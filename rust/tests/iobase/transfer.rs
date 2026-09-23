//! `rust/src/iobase/transfer.rs`: a resumable write session shapes every
//! chunk it is handed onto the one shape its first chunk settled.

use std::sync::Arc;

use arrow_array::{ArrayRef, Int32Array, Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema, SchemaRef};

use yggdryl::holder::Buffer;
use yggdryl::media::IORecordOptions;
use yggdryl::{ArrowWriteSession, DataType, Field, IOMedia, StructType, Url};

fn schema() -> Field {
    StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("row")
}

/// One chunk of `ids`, laid out as `schema` lays its key out.
fn chunk(schema: &SchemaRef, ids: &[i64]) -> RecordBatch {
    let id: ArrayRef = match schema.field(0).data_type() {
        ArrowDataType::Int32 => Arc::new(Int32Array::from(
            ids.iter()
                .map(|id| i32::try_from(*id).unwrap())
                .collect::<Vec<_>>(),
        )),
        _ => Arc::new(Int64Array::from(ids.to_vec())),
    };
    let symbols: Vec<String> = ids.iter().map(|id| format!("S{id}")).collect();
    RecordBatch::try_new(
        Arc::clone(schema),
        vec![id, Arc::new(StringArray::from(symbols))],
    )
    .unwrap()
}

#[test]
fn a_later_chunk_of_another_layout_is_shaped_onto_the_first() {
    let mut handle = Buffer::new().with_media_type(
        Url::from_str("file:///session-layouts.arrows")
            .unwrap()
            .media_type(),
    );
    let options = handle
        .record_options()
        .unwrap()
        .with_field(schema())
        .with_filter("id > 1")
        .unwrap()
        .with_commit_row_size(2);
    let declared = schema().into_arrow_schema().unwrap();
    // The same columns laid out another way: a narrower key admitting nulls.
    let narrow = Arc::new(Schema::new(vec![
        ArrowField::new("id", ArrowDataType::Int32, true),
        ArrowField::new("symbol", ArrowDataType::Utf8, true),
    ]));

    // A layout that returns after another is shaped as it was the first time.
    let mut session = ArrowWriteSession::overwrite(&options).unwrap();
    for (layout, ids) in [
        (&declared, &[1, 2][..]),
        (&narrow, &[3, 4]),
        (&narrow, &[5]),
        (&declared, &[6]),
    ] {
        let batches = yggdryl::arrow::batch_reader(Arc::clone(layout), [chunk(layout, ids)]);
        assert!(session.push(&mut handle, batches).unwrap());
    }
    session.finish(&mut handle).unwrap();

    let read = handle.record_options().unwrap().with_field(schema());
    assert_eq!(handle.read_arrow_field(&read).unwrap(), schema());
    let mut ids = Vec::new();
    for batch in handle.read_arrow_reader(&read).unwrap() {
        let batch = batch.unwrap();
        let column = batch
            .column(0)
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("the declared Int64 key");
        ids.extend(column.values().iter().copied());
    }
    assert_eq!(ids, vec![2, 3, 4, 5, 6]);
}
