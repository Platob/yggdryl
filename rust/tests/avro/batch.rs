//! `rust/src/avro/batch.rs`: the column decoders no caller can name.
//!
//! One decoder is built per column, so a variant payload widening the shared
//! enum is paid for by every column in the file. Both enums are file-private,
//! and only their width is pinned, so each is measured through
//! `yggdryl::internals`.

#[cfg(feature = "internals")]
mod internal {
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
}

mod avro {
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::{MediaType, MimeType};

    /// Append a zig-zag variable-length integer, as Avro's `long` is encoded.
    ///
    /// Spelled here rather than borrowed from the codec: a fixture that builds its
    /// bytes with the reader under test proves only that the two agree.
    fn put_long(target: &mut Vec<u8>, value: i64) {
        let mut encoded = ((value << 1) ^ (value >> 63)) as u64;
        loop {
            let byte = u8::try_from(encoded & 0x7f).unwrap_or_default();
            encoded >>= 7;
            if encoded == 0 {
                target.push(byte);
                return;
            }
            target.push(byte | 0x80);
        }
    }

    /// Append a length-prefixed byte run, as Avro's `bytes` is encoded.
    fn put_bytes(target: &mut Vec<u8>, bytes: &[u8]) {
        put_long(target, bytes.len() as i64);
        target.extend_from_slice(bytes);
    }

    fn buffer() -> Buffer {
        let mut buffer = Buffer::new();
        buffer.set_media_type(MediaType::new(MimeType::AVRO));
        buffer
    }

    /// Write a container by hand: magic, header, one block per payload.
    fn handmade_container(schema_json: &str, codec: &str, blocks: &[(i64, Vec<u8>)]) -> Buffer {
        handmade_container_with_header(
            &[
                ("avro.schema", schema_json.as_bytes()),
                ("avro.codec", codec.as_bytes()),
            ],
            blocks,
        )
    }

    /// Write a container with caller-controlled header entries for hardening tests.
    fn handmade_container_with_header(
        entries: &[(&str, &[u8])],
        blocks: &[(i64, Vec<u8>)],
    ) -> Buffer {
        let mut output = Vec::new();
        output.extend_from_slice(b"Obj\x01");
        put_long(&mut output, entries.len() as i64);
        for (key, value) in entries {
            put_bytes(&mut output, key.as_bytes());
            put_bytes(&mut output, value);
        }
        put_long(&mut output, 0);
        let sync = [7_u8; 16];
        output.extend_from_slice(&sync);
        for (count, payload) in blocks {
            put_long(&mut output, *count);
            put_bytes(&mut output, payload);
            output.extend_from_slice(&sync);
        }
        let mut handle = buffer();
        handle.write_all_bytes(&output).unwrap();
        handle
    }

    mod records {

        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::{Arc, Mutex};
        use yggdryl::StructType;
        use yggdryl::avro::Avro;

        use arrow_array::builder::{Float64Builder, Int64Builder, ListBuilder, StringBuilder};
        use arrow_array::types::{Float64Type, Int64Type};
        use arrow_array::{Array, RecordBatch, RecordBatchIterator, cast::AsArray};
        use arrow_schema::ArrowError;

        use yggdryl::avro;
        use yggdryl::avro::AvroOptions;
        use yggdryl::holder::Buffer;
        use yggdryl::media::{IORecordOptions, RecordOptions};
        use yggdryl::{DataType, DataTypeId, Field, MediaType, Scalar, TimeUnit, Url};
        use yggdryl::{IOBase, IOMedia};

        /// One canonical batch with a nullable column and a list column.
        fn batch() -> (Field, RecordBatch) {
            let schema = Field::new(
                "trades",
                StructType::from_fields([
                    DataType::Int64.required_field("id"),
                    DataType::utf8().nullable_field("symbol"),
                    DataType::Float64.nullable_field("price"),
                    DataType::serie(DataType::Int64.required_field("item")).required_field("legs"),
                ])
                .map(DataType::from)
                .unwrap(),
                false,
            );
            let mut symbols = StringBuilder::new();
            symbols.append_value("AAPL");
            symbols.append_null();
            symbols.append_value("MSFT");
            let mut prices = Float64Builder::new();
            prices.append_value(187.5);
            prices.append_value(12.25);
            prices.append_null();
            let mut legs = ListBuilder::new(Int64Builder::new());
            legs.append_value([Some(1), Some(2)]);
            legs.append_value([]);
            legs.append_value([Some(7)]);
            let legs = {
                // The canonical list item is a required field called `item`.
                let array = legs.finish();
                let (_, offsets, values, nulls) = array.into_parts();
                arrow_array::ListArray::new(
                    Arc::new(arrow_schema::Field::new(
                        "item",
                        arrow_schema::DataType::Int64,
                        false,
                    )),
                    offsets,
                    values,
                    nulls,
                )
            };
            let arrow_schema = schema.clone().into_arrow_schema().unwrap();
            let batch = RecordBatch::try_new(
                arrow_schema,
                vec![
                    Arc::new(arrow_array::Int64Array::from(vec![1, 2, 3])),
                    Arc::new(symbols.finish()),
                    Arc::new(prices.finish()),
                    Arc::new(legs),
                ],
            )
            .unwrap();
            (schema, batch)
        }

        /// One batch's rows, each the positional sequence of its columns'
        /// values: the batch landed as a column of its own schema and held as
        /// one value, then its rows built.
        fn rows_of(batch: &RecordBatch) -> Vec<Scalar> {
            let landed =
                yggdryl::Serie::from_arrow_batch(None, batch, yggdryl::ArrowCastOptions::default())
                    .unwrap();
            Scalar::from(landed).sequence_rows().unwrap().into_owned()
        }

        fn handle() -> Buffer {
            Buffer::new()
                .with_media_type(Url::from_str("file:///trades.avro").unwrap().media_type())
        }

        /// Two independent handles over one in-memory byte value.
        #[derive(Clone, Debug)]
        struct Shared {
            handle: Arc<Mutex<Buffer>>,
            media_type: MediaType,
        }

        impl Shared {
            fn new(handle: Buffer) -> Self {
                let media_type = handle.media_type().clone();
                Self {
                    handle: Arc::new(Mutex::new(handle)),
                    media_type,
                }
            }
        }

        impl yggdryl::IOMedia for Shared {
            yggdryl::impl_default_iomedia!();
        }

        impl IOBase for Shared {
            fn pread(&self, offset: u64, buffer: &mut [u8]) -> yggdryl::Result<usize> {
                self.handle.lock().unwrap().pread(offset, buffer)
            }

            fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> yggdryl::Result<usize> {
                self.handle.lock().unwrap().pwrite(offset, bytes)
            }

            fn create_bytes(&mut self, bytes: &[u8]) -> yggdryl::Result<()> {
                self.handle.lock().unwrap().create_bytes(bytes)
            }

            fn size(&self) -> u64 {
                self.handle.lock().unwrap().size()
            }

            fn capacity(&self) -> u64 {
                self.handle.lock().unwrap().capacity()
            }

            fn reserve(&mut self, capacity: u64) -> yggdryl::Result<()> {
                self.handle.lock().unwrap().reserve(capacity)
            }

            fn truncate(&mut self, size: u64) -> yggdryl::Result<()> {
                self.handle.lock().unwrap().truncate(size)
            }

            fn uri(&self) -> Option<&yggdryl::Uri> {
                None
            }

            fn url(&self) -> Option<&Url> {
                None
            }

            fn media_type(&self) -> &MediaType {
                &self.media_type
            }

            fn set_media_type(&mut self, media_type: MediaType) {
                self.handle
                    .lock()
                    .unwrap()
                    .set_media_type(media_type.clone());
                self.media_type = media_type;
            }
        }

        /// A positional handle measuring how much row-size metadata traversal
        /// fetches from a large container.
        struct Counting {
            handle: Buffer,
            reads: AtomicUsize,
            bytes: AtomicUsize,
        }

        impl Counting {
            fn new(handle: Buffer) -> Self {
                Self {
                    handle,
                    reads: AtomicUsize::new(0),
                    bytes: AtomicUsize::new(0),
                }
            }

            fn cost(&self, operation: impl FnOnce()) -> (usize, usize) {
                let reads = self.reads.load(Ordering::Relaxed);
                let bytes = self.bytes.load(Ordering::Relaxed);
                operation();
                (
                    self.reads.load(Ordering::Relaxed) - reads,
                    self.bytes.load(Ordering::Relaxed) - bytes,
                )
            }
        }

        impl yggdryl::IOMedia for Counting {
            yggdryl::impl_default_iomedia!();
        }

        impl IOBase for Counting {
            yggdryl::delegate_iobase!(handle: create_bytes, pwrite, size, capacity, reserve,
                truncate, uri, url, media_type, set_media_type, flush, parent, child_by_path,
                ls, kind, clear, remove, is_atomic, is_tabular);

            fn pread(&self, offset: u64, buffer: &mut [u8]) -> yggdryl::Result<usize> {
                let read = self.handle.pread(offset, buffer)?;
                self.reads.fetch_add(1, Ordering::Relaxed);
                self.bytes.fetch_add(read, Ordering::Relaxed);
                Ok(read)
            }
        }

        /// A one-batch reader that reports whether a write pulled its input.
        fn counted_reader(pulls: Arc<AtomicUsize>) -> yggdryl::arrow::BatchReader {
            let (_, batch) = batch();
            let schema = batch.schema();
            let batches = std::iter::once(batch).inspect(move |_| {
                pulls.fetch_add(1, Ordering::Relaxed);
            });
            yggdryl::arrow::batch_reader(schema, batches)
        }

        fn reader_then_error(first: RecordBatch) -> yggdryl::arrow::BatchReader {
            let schema = first.schema();
            Box::new(RecordBatchIterator::new(
                [
                    Ok(first),
                    Err(ArrowError::ComputeError("later Avro source failure".into())),
                ],
                schema,
            ))
        }

        #[test]
        fn batches_round_trip_through_the_record_surface() {
            let (schema, batch) = batch();
            let mut handle = handle();
            let options =
                yggdryl::media::RecordOptions::for_media_type(handle.media_type()).unwrap();
            handle
                .overwrite_arrow_reader(
                    yggdryl::arrow::batch_reader(batch.schema(), [batch.clone()]),
                    &options,
                )
                .unwrap();

            let read: Vec<RecordBatch> = handle
                .read_arrow_reader(&options)
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert_eq!(read.len(), 1);
            assert_eq!(read[0].num_rows(), 3);
            assert_eq!(
                read[0].column(0).as_primitive::<Int64Type>().values(),
                &[1, 2, 3]
            );
            let symbols = read[0].column(1).as_string::<i32>();
            assert_eq!(symbols.value(0), "AAPL");
            assert!(symbols.is_null(1));
            let legs = read[0].column(3).as_list::<i32>();
            assert_eq!(legs.value(0).len(), 2);
            assert_eq!(legs.value(1).len(), 0);
            let _ = schema;
        }

        #[test]
        fn record_batches_keep_exact_avro_logical_and_fixed_leaves() {
            let field = StructType::from_fields([
                DataType::uuid().required_field("id"),
                DataType::decimal32(9, 2).unwrap().required_field("small"),
                DataType::decimal64(18, 2).unwrap().required_field("large"),
                DataType::fixed_binary(3).unwrap().required_field("raw"),
                DataType::interval(TimeUnit::MonthDayNano)
                    .unwrap()
                    .required_field("span"),
            ])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
            let row = Scalar::from_struct([
                ("id", Scalar::from("00112233-4455-6677-8899-aabbccddeeff")),
                ("small", Scalar::decimal128(123, 2)),
                ("large", Scalar::decimal128(456, 2)),
                ("raw", Scalar::from([1_u8, 2, 3].as_slice())),
                (
                    "span",
                    Scalar::from_sequence([
                        Scalar::from(1),
                        Scalar::from(2),
                        Scalar::from(3_000_000),
                    ]),
                ),
            ])
            .unwrap();
            let batch = yggdryl::Serie::from_scalars(field, [row])
                .unwrap()
                .into_arrow_batch()
                .unwrap();
            let mut handle = handle();
            avro::overwrite_arrow_reader(
                &mut handle,
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &AvroOptions::new(),
            )
            .unwrap();

            let read_field = avro::read_field(&handle, &AvroOptions::new()).unwrap();
            let ids = read_field
                .fields()
                .iter()
                .map(|field| field.id())
                .collect::<Vec<_>>();
            assert_eq!(
                ids,
                [
                    DataTypeId::Uuid,
                    DataTypeId::Decimal32,
                    DataTypeId::Decimal64,
                    DataTypeId::FixedBinary,
                    DataTypeId::Interval,
                ]
            );

            let batches = avro::read_batch_reader(&handle, None, &AvroOptions::new())
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            let values = rows_of(&batches[0]);
            let row = values[0].as_sequence().unwrap();
            assert_eq!(row.iter().map(Scalar::id).collect::<Vec<_>>(), ids);
        }

        #[test]
        fn batches_changing_layout_mid_stream_each_reconcile_to_the_container_schema() {
            let canonical = Arc::new(arrow_schema::Schema::new(vec![
                arrow_schema::Field::new("id", arrow_schema::DataType::Int64, false),
                arrow_schema::Field::new("symbol", arrow_schema::DataType::Utf8, true),
            ]));
            let narrow = Arc::new(arrow_schema::Schema::new(vec![
                arrow_schema::Field::new("id", arrow_schema::DataType::Int32, false),
                arrow_schema::Field::new("symbol", arrow_schema::DataType::LargeUtf8, true),
            ]));
            let exact = |ids: Vec<i64>, symbol: &str| {
                RecordBatch::try_new(
                    Arc::clone(&canonical),
                    vec![
                        Arc::new(arrow_array::Int64Array::from(ids.clone())),
                        Arc::new(arrow_array::StringArray::from(vec![symbol; ids.len()])),
                    ],
                )
                .unwrap()
            };
            let other = RecordBatch::try_new(
                Arc::clone(&narrow),
                vec![
                    Arc::new(arrow_array::Int32Array::from(vec![3, 4])),
                    Arc::new(arrow_array::LargeStringArray::from(vec![
                        None,
                        Some("MSFT"),
                    ])),
                ],
            )
            .unwrap();
            let mut handle = handle();
            avro::overwrite_arrow_reader(
                &mut handle,
                yggdryl::arrow::batch_reader(
                    Arc::clone(&canonical),
                    [
                        exact(vec![1, 2], "AAPL"),
                        other.clone(),
                        exact(vec![5], "IBM"),
                        other,
                    ],
                ),
                &AvroOptions::new(),
            )
            .unwrap();

            let batches = avro::read_batch_reader(&handle, None, &AvroOptions::new())
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            let ids: Vec<i64> = batches
                .iter()
                .flat_map(|batch| {
                    batch
                        .column(0)
                        .as_primitive::<Int64Type>()
                        .values()
                        .to_vec()
                })
                .collect();
            assert_eq!(ids, [1, 2, 3, 4, 5, 3, 4]);
            let symbols: Vec<Option<String>> = batches
                .iter()
                .flat_map(|batch| {
                    let column = batch.column(1).as_string::<i32>();
                    (0..column.len())
                        .map(|row| column.is_valid(row).then(|| column.value(row).to_owned()))
                        .collect::<Vec<_>>()
                })
                .collect();
            assert_eq!(
                symbols,
                [
                    Some("AAPL"),
                    Some("AAPL"),
                    None,
                    Some("MSFT"),
                    Some("IBM"),
                    None,
                    Some("MSFT")
                ]
                .map(|symbol| symbol.map(str::to_owned))
            );
        }

        #[test]
        fn a_variant_column_writes_the_record_the_specification_states() {
            // Avro spells a variant as a record of `metadata` and `value`, both
            // `bytes`, read by name and carrying no field ids. The annotation
            // beside them is what names it one on the way back.
            let field = StructType::from_fields([
                DataType::Int64.required_field("id"),
                DataType::variant().required_field("payload"),
            ])
            .map(DataType::from)
            .unwrap()
            .required_field("row");
            let payload = Scalar::from_struct([
                ("symbol", Scalar::from("AAPL")),
                ("size", Scalar::from(100_i64)),
            ])
            .unwrap();
            let row =
                Scalar::from_struct([("id", Scalar::from(1_i64)), ("payload", payload.clone())])
                    .unwrap();
            let batch = yggdryl::Serie::from_scalars(field, [row])
                .unwrap()
                .into_arrow_batch()
                .unwrap();
            let mut handle = handle();
            avro::overwrite_arrow_reader(
                &mut handle,
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &AvroOptions::new(),
            )
            .unwrap();

            // The written schema is the record the specification states.
            let written = avro::read_container(&handle).unwrap().schema.into_json();
            let columns = written
                .get_key_str("fields")
                .unwrap()
                .as_sequence()
                .unwrap();
            let variant = columns[1].get_key_str("type").unwrap();
            assert_eq!(
                variant.get_key_str("type").unwrap().as_str(),
                Some("record")
            );
            assert_eq!(
                variant.get_key_str("logicalType").unwrap().as_str(),
                Some("variant")
            );
            let children = variant
                .get_key_str("fields")
                .unwrap()
                .as_sequence()
                .unwrap();
            assert_eq!(children.len(), 2);
            for (child, name) in children.iter().zip(["metadata", "value"]) {
                assert_eq!(child.get_key_str("name").unwrap().as_str(), Some(name));
                assert_eq!(child.get_key_str("type").unwrap().as_str(), Some("bytes"));
                assert!(child.get_key_str("field-id").is_none(), "{child:?}");
            }

            // And the column reads back as a variant holding the value written.
            let read_field = avro::read_field(&handle, &AvroOptions::new()).unwrap();
            assert_eq!(read_field.fields()[1].dtype(), &DataType::variant());
            let batches = avro::read_batch_reader(&handle, None, &AvroOptions::new())
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            let values = rows_of(&batches[0]);
            let row = values[0].as_sequence().unwrap();
            let Scalar::Variant(held) = &row[1] else {
                panic!("a variant value, got {:?}", row[1]);
            };
            assert_eq!(held.scalar().unwrap(), payload);
        }

        #[test]
        fn a_variant_record_reads_its_two_bytes_by_name_in_either_schema_order() {
            // Avro records encode in schema order, but the variant contract names
            // its two byte fields. This fixture puts `value` first on the wire.
            let payload = Scalar::from_struct([("symbol", Scalar::from("AAPL"))]).unwrap();
            let variant = yggdryl::Variant::encode(&payload).unwrap();
            let mut datum = Vec::new();
            super::put_bytes(&mut datum, variant.value());
            super::put_bytes(&mut datum, variant.metadata());
            let foreign = super::handmade_container(
                r#"{"type":"record","name":"row","fields":[
                {"name":"payload","type":{"type":"record","name":"payload_value","logicalType":"variant","fields":[
                    {"name":"value","type":"bytes"},
                    {"name":"metadata","type":"bytes"}
                ]}}
            ]}"#,
                "null",
                &[(1, datum)],
            );

            let field = avro::read_field(&foreign, &AvroOptions::new()).unwrap();
            assert_eq!(field.fields()[0].dtype(), &DataType::variant());
            let native = avro::read_container(&foreign).unwrap();
            let Scalar::Variant(read_native) = &native.rows[0].get_key_str("payload").unwrap()
            else {
                panic!("a native variant value, got {:?}", native.rows[0]);
            };
            assert_eq!(read_native.metadata(), variant.metadata());
            assert_eq!(read_native.value(), variant.value());
            let mut rewritten = handle();
            avro::write_container(
                &mut rewritten,
                &native.schema.clone().into_json(),
                &[],
                &native.rows,
            )
            .unwrap();
            assert_eq!(
                avro::read_container(&rewritten).unwrap().rows,
                native.rows,
                "native datum write retains the two buffers in reversed schema order"
            );
            let union_schema = yggdryl::json::from_utf8(
                r#"{"type":"record","name":"row","fields":[{"name":"payload","type":["null",{"type":"record","name":"payload_value","logicalType":"variant","fields":[
                {"name":"value","type":"bytes"},
                {"name":"metadata","type":"bytes"}
            ]}]}]}"#,
            )
            .unwrap();
            let union_row =
                Scalar::from_struct([("payload", Scalar::Variant(variant.clone()))]).unwrap();
            let absent = Scalar::from_struct([("payload", Scalar::Null)]).unwrap();
            let encoded_null = Scalar::from_struct([(
                "payload",
                Scalar::Variant(Scalar::Null.into_variant().unwrap()),
            )])
            .unwrap();
            let union_rows = [union_row, absent, encoded_null];
            let mut union = handle();
            avro::write_container(&mut union, &union_schema, &[], &union_rows).unwrap();
            assert_eq!(avro::read_container(&union).unwrap().rows, union_rows);
            let union_batch = avro::read_batch_reader(&union, None, &AvroOptions::new())
                .unwrap()
                .next()
                .unwrap()
                .unwrap();
            let rows = rows_of(&union_batch);
            assert!(matches!(
                rows[0].as_sequence().unwrap()[0],
                Scalar::Variant(_)
            ));
            assert!(rows[1].as_sequence().unwrap()[0].is_null());
            let Scalar::Variant(null) = &rows[2].as_sequence().unwrap()[0] else {
                panic!("encoded null remains a present Variant");
            };
            assert!(null.scalar().unwrap().is_null());
            let batch = avro::read_batch_reader(&foreign, None, &AvroOptions::new())
                .unwrap()
                .next()
                .unwrap()
                .unwrap();
            let batch_schema = batch.schema();
            let payload_field = batch_schema.field_with_name("payload").unwrap();
            let arrow_schema::DataType::Struct(children) = payload_field.data_type() else {
                panic!("the variant's two-buffer storage");
            };
            assert_eq!(
                children
                    .iter()
                    .map(|child| child.name())
                    .collect::<Vec<_>>(),
                ["metadata", "value"]
            );
            let values = rows_of(&batch);
            let Scalar::Variant(read) = &values[0].as_sequence().unwrap()[0] else {
                panic!("a variant value, got {values:?}");
            };
            assert_eq!(read.scalar().unwrap(), payload);
        }

        #[test]
        fn a_projection_skips_the_bytes_of_unselected_columns() {
            let (_, batch) = batch();
            let mut handle = handle();
            let options = AvroOptions::new();
            avro::overwrite_arrow_reader(
                &mut handle,
                yggdryl::arrow::batch_reader(batch.schema(), [batch.clone()]),
                &options,
            )
            .unwrap();

            let narrow = Field::new(
                "trades",
                StructType::from_fields([
                    DataType::Int64.required_field("id"),
                    DataType::Float64.nullable_field("price"),
                ])
                .map(DataType::from)
                .unwrap(),
                false,
            );
            let read: Vec<RecordBatch> = avro::read_batch_reader(&handle, Some(&narrow), &options)
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert_eq!(read[0].num_columns(), 2, "{:?}", read[0].schema());
            assert_eq!(
                read[0].column(0).as_primitive::<Int64Type>().values(),
                &[1, 2, 3]
            );
            assert_eq!(
                read[0].column(1).as_primitive::<Float64Type>().value(0),
                187.5
            );
            assert!(read[0].column(1).as_primitive::<Float64Type>().is_null(2));
        }

        #[test]
        fn an_empty_handle_reads_as_no_batches() {
            let handle = handle();
            let options = AvroOptions::new().with_field(batch().0);
            let read = avro::read_batch_reader(&handle, None, &options).unwrap();
            assert_eq!(read.count(), 0);
        }

        #[test]
        fn dimensions_describe_all_blocks_and_ignore_read_options() {
            let (field, first) = batch();
            let (_, second) = batch();
            let mut media = Avro::new(handle());
            let options = media.record_options().unwrap();
            media
                .overwrite_arrow_reader(
                    yggdryl::arrow::batch_reader(first.schema(), [first, second]),
                    &options,
                )
                .unwrap();
            media.options_mut().set_max_row_size(Some(1));
            media.options_mut().set_select("id".parse().unwrap());
            media
                .options_mut()
                .set_filter("id = '999'".parse().unwrap());

            assert_eq!(media.row_size().unwrap(), 6);
            assert_eq!(media.column_size().unwrap(), field.field_len());
        }

        #[test]
        fn an_empty_open_avro_container_has_explicit_lifecycle_and_dimensions() {
            let (field, batch) = batch();
            let mut media = Avro::new(handle()).with_field(field.clone());

            media.open().unwrap();
            assert!(media.opened());
            assert_eq!(media.row_size().unwrap(), 0);
            assert_eq!(media.column_size().unwrap(), field.field_len());

            let options = media.record_options().unwrap();
            media
                .overwrite_arrow_reader(
                    yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                    &options,
                )
                .unwrap();
            assert!(media.opened());
            assert_eq!(media.row_size().unwrap(), 3);

            media.clear().unwrap();
            assert!(media.opened());
            assert_eq!(media.row_size().unwrap(), 0);
            assert_eq!(media.column_size().unwrap(), field.field_len());

            media.remove(false).unwrap();
            assert!(!media.opened(), "removal ends the opened session");
        }

        #[test]
        fn an_open_avro_cache_is_stable_until_close_then_reads_fresh() {
            let encoded = |copies: usize| {
                let (_, template) = batch();
                let schema = template.schema();
                let batches = std::iter::repeat_n(template, copies);
                let mut media = Avro::new(handle());
                let options = media.record_options().unwrap();
                media
                    .overwrite_arrow_reader(yggdryl::arrow::batch_reader(schema, batches), &options)
                    .unwrap();
                media.into_handle()
            };
            let first = encoded(1);
            let replacement = encoded(2);
            let shared = Shared::new(first);
            let mut external = shared.clone();
            let mut media = Avro::new(shared);

            media.open().unwrap();
            assert_eq!(media.row_size().unwrap(), 3);
            external.write_all_bytes(replacement.as_slice()).unwrap();
            assert_eq!(media.row_size().unwrap(), 3, "the open metadata is stable");

            media.close().unwrap();
            assert!(!media.opened());
            assert_eq!(media.row_size().unwrap(), 6, "closed reads are fresh");
        }

        #[test]
        fn dimensions_skip_a_large_avro_payload_without_decoding_it() {
            let field = StructType::from_fields([DataType::utf8().required_field("payload")])
                .map(DataType::from)
                .unwrap()
                .required_field("rows");
            let payload = "0123456789abcdef".repeat(65_536);
            let batch = RecordBatch::try_new(
                field.clone().into_arrow_schema().unwrap(),
                vec![Arc::new(arrow_array::StringArray::from(vec![payload]))],
            )
            .unwrap();
            let options = AvroOptions::new()
                .with_codec("null")
                .with_field(field.clone());
            let mut media = Avro::new(handle()).with_options(options);
            let options = media.record_options().unwrap();
            media
                .overwrite_arrow_reader(
                    yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                    &options,
                )
                .unwrap();
            let counting = Counting::new(media.into_handle());
            let total = counting.size() as usize;

            let (schema_reads, schema_fetched) = counting.cost(|| {
                assert_eq!(counting.column_size().unwrap(), 1);
            });
            assert!(schema_reads > 0);
            assert!(
                schema_fetched * 4 < total,
                "schema header fetched {schema_fetched} of {total} encoded bytes"
            );

            let (reads, fetched) = counting.cost(|| {
                assert_eq!(counting.row_size().unwrap(), 1);
            });
            assert!(reads > 0);
            assert!(
                fetched * 4 < total,
                "metadata traversal fetched {fetched} of {total} encoded bytes"
            );
        }

        #[test]
        fn an_outer_content_coding_is_rejected_by_name() {
            let handle = Buffer::new().with_media_type(
                Url::from_str("file:///trades.avro.gz")
                    .unwrap()
                    .media_type(),
            );
            let message = avro::read_field(&handle, &AvroOptions::new())
                .unwrap_err()
                .to_string();
            assert!(message.contains("outer content coding"), "{message}");
            assert!(message.contains("gzip"), "{message}");
        }

        #[test]
        fn the_stateful_wrapper_caches_the_schema_between_open_and_close() {
            let (schema, batch) = batch();
            // An Arrow schema is anonymous, so the record's name is the options'
            // root name; naming it keeps the round trip exact.
            let mut media = Avro::new(handle()).with_name("trades");
            let options = media.record_options().unwrap();
            media
                .overwrite_arrow_reader(
                    yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                    &options,
                )
                .unwrap();
            assert!(!media.opened(), "a closed write must not start a session");
            media.open().unwrap();
            assert!(media.opened());
            let derived = media.read_arrow_field(&options).unwrap();
            assert_eq!(derived.name(), schema.name());
            assert_eq!(derived.field_len(), schema.field_len());
            media.close().unwrap();
            assert!(!media.opened());
        }

        #[test]
        fn the_wrapper_owns_avro_options_over_an_unnamed_buffer() {
            let (field, _) = batch();
            let media = Avro::new(Buffer::new()).with_field(field.clone());
            let options = media.record_options().unwrap();

            assert!(options.settings::<AvroOptions>().is_some());
            assert_eq!(options.field(), Some(field));
        }

        #[test]
        fn mismatched_options_are_rejected_before_any_write_pulls_input() {
            let (field, _) = batch();
            for operation in ["overwrite", "append", "merge"] {
                let pulls = Arc::new(AtomicUsize::new(0));
                let mut media = Avro::new(Buffer::new()).with_field(field.clone());
                let mut options = RecordOptions::Ipc(yggdryl::ipc::IpcOptions::new());
                if operation == "merge" {
                    options.set_merge_by(yggdryl::expression::Selector::from_columns(["id"]));
                }
                let result = match operation {
                    "overwrite" => yggdryl::IOMedia::overwrite_arrow_reader(
                        &mut media,
                        counted_reader(Arc::clone(&pulls)),
                        &options,
                    ),
                    "append" => yggdryl::IOMedia::append_arrow_reader(
                        &mut media,
                        counted_reader(Arc::clone(&pulls)),
                        &options,
                    ),
                    "merge" => yggdryl::IOMedia::merge_arrow_reader(
                        &mut media,
                        counted_reader(Arc::clone(&pulls)),
                        &options,
                    ),
                    _ => unreachable!(),
                };

                let message = result.unwrap_err().to_string();
                assert!(message.contains("Avro"), "{operation}: {message}");
                assert_eq!(pulls.load(Ordering::Relaxed), 0, "{operation}");
                assert!(media.handle().is_empty(), "{operation}");
            }
        }

        #[test]
        fn an_open_cache_tracks_the_final_avro_field_after_casting() {
            let stored = StructType::from_fields([
                DataType::Int64.required_field("id"),
                DataType::utf8().nullable_field("symbol"),
            ])
            .map(DataType::from)
            .unwrap()
            .required_field("trades");
            let first = RecordBatch::try_new(
                stored.clone().into_arrow_schema().unwrap(),
                vec![
                    Arc::new(arrow_array::Int64Array::from(vec![1])),
                    Arc::new(arrow_array::StringArray::from(vec![Some("AAPL")])),
                ],
            )
            .unwrap();
            let mut media = Avro::new(handle()).with_name("trades");
            let options = media.record_options().unwrap();
            media
                .overwrite_arrow_reader(
                    yggdryl::arrow::batch_reader(first.schema(), [first]),
                    &options,
                )
                .unwrap();
            media.open().unwrap();

            let loose = StructType::from_fields([
                DataType::utf8().required_field("id"),
                DataType::utf8().nullable_field("symbol"),
            ])
            .map(DataType::from)
            .unwrap()
            .required_field("trades");
            let incoming = RecordBatch::try_new(
                loose.clone().into_arrow_schema().unwrap(),
                vec![
                    Arc::new(arrow_array::StringArray::from(vec!["7"])),
                    Arc::new(arrow_array::StringArray::from(vec![Some("MSFT")])),
                ],
            )
            .unwrap();
            media
                .overwrite_arrow_reader(
                    yggdryl::arrow::batch_reader(incoming.schema(), [incoming]),
                    &options,
                )
                .unwrap();

            assert!(media.opened());
            assert_eq!(media.read_arrow_field(&options).unwrap(), stored);
        }

        #[test]
        fn append_and_merge_keep_an_open_avro_cache_coherent_until_close() {
            let (field, first) = batch();
            let mut media = Avro::new(Buffer::new()).with_field(field.clone());
            let options = media.record_options().unwrap();
            media
                .overwrite_arrow_reader(
                    yggdryl::arrow::batch_reader(first.schema(), [first]),
                    &options,
                )
                .unwrap();
            assert!(!media.opened());
            media.open().unwrap();
            media.options_mut().set_commit_batch_num(Some(1));

            let (_, appended) = batch();
            let append_options = media.record_options().unwrap();
            media
                .append_arrow_reader(
                    yggdryl::arrow::batch_reader(appended.schema(), [appended]),
                    &append_options,
                )
                .unwrap();
            assert!(media.opened());
            assert_eq!(media.read_arrow_field(&append_options).unwrap(), field);
            assert_eq!(
                media
                    .read_arrow_reader(&append_options)
                    .unwrap()
                    .map(|batch| batch.unwrap().num_rows())
                    .sum::<usize>(),
                6
            );

            media
                .options_mut()
                .set_merge_by(yggdryl::expression::Selector::from_columns(["id"]));
            let (_, merged) = batch();
            let merge_options = media.record_options().unwrap();
            media
                .merge_arrow_reader(
                    yggdryl::arrow::batch_reader(merged.schema(), [merged]),
                    &merge_options,
                )
                .unwrap();
            assert!(media.opened());
            assert_eq!(media.read_arrow_field(&merge_options).unwrap(), field);
            assert_eq!(
                media
                    .read_arrow_reader(&merge_options)
                    .unwrap()
                    .map(|batch| batch.unwrap().num_rows())
                    .sum::<usize>(),
                6
            );

            media.close().unwrap();
            assert!(!media.opened());
            assert_eq!(media.read_arrow_field(&merge_options).unwrap(), field);
        }

        #[test]
        fn a_partial_commit_keeps_an_open_avro_cache_coherent() {
            let (field, first) = batch();
            let mut media = Avro::new(Buffer::new()).with_field(field.clone());
            let options = media.record_options().unwrap();
            media
                .overwrite_arrow_reader(
                    yggdryl::arrow::batch_reader(first.schema(), [first]),
                    &options,
                )
                .unwrap();
            media.open().unwrap();
            media.options_mut().set_commit_batch_num(Some(1));
            let (_, incoming) = batch();

            let options = media.record_options().unwrap();
            let message = media
                .overwrite_arrow_reader(reader_then_error(incoming.slice(0, 1)), &options)
                .unwrap_err()
                .to_string();

            assert!(message.contains("later Avro source failure"), "{message}");
            assert!(media.opened());
            assert_eq!(media.read_arrow_field(&options).unwrap(), field);
            assert_eq!(media.row_size().unwrap(), 1);
            assert_eq!(
                media
                    .read_arrow_reader(&options)
                    .unwrap()
                    .map(|batch| batch.unwrap().num_rows())
                    .sum::<usize>(),
                1
            );
        }

        #[test]
        fn a_wide_union_is_refused_for_the_record_surface() {
            let mut handle = handle();
            avro::write_container(
                &mut handle,
                &yggdryl::json::from_utf8(
                    r#"{"type":"record","name":"row","fields":[
                    {"name":"v","type":["null","long","string"]}
                ]}"#,
                )
                .unwrap(),
                &[],
                &[],
            )
            .unwrap();
            let message = avro::read_field(&handle, &AvroOptions::new())
                .unwrap_err()
                .to_string();
            assert!(message.contains("3 branches"), "{message}");
        }

        #[test]
        fn single_branch_unions_read_through_the_record_surface() {
            // ["null"] alone and ["long"] alone are legal unions, and each still
            // spends a branch index on the wire.
            let mut handle = handle();
            avro::write_container(
                &mut handle,
                &yggdryl::json::from_utf8(
                    r#"{"type":"record","name":"row","fields":[
                    {"name":"gap","type":["null"]},
                    {"name":"v","type":["long"]}
                ]}"#,
                )
                .unwrap(),
                &[],
                &[
                    yggdryl::json::from_utf8(r#"{"gap":null,"v":5}"#).unwrap(),
                    yggdryl::json::from_utf8(r#"{"gap":null,"v":6}"#).unwrap(),
                ],
            )
            .unwrap();
            let read: Vec<RecordBatch> =
                avro::read_batch_reader(&handle, None, &AvroOptions::new())
                    .unwrap()
                    .collect::<Result<_, _>>()
                    .unwrap();
            assert_eq!(read[0].num_rows(), 2);
            // A NullArray's nulls are logical, so the count is asked logically.
            assert_eq!(read[0].column(0).logical_null_count(), 2);
            assert_eq!(
                read[0].column(1).as_primitive::<Int64Type>().values(),
                &[5, 6]
            );
        }

        #[test]
        fn a_null_typed_column_round_trips_without_double_wrapping() {
            // A nullable Null column must not be spelled ["null","null"] - the
            // declared type is already the null the wrap would add.
            let schema = Field::new(
                "row",
                StructType::from_fields([
                    DataType::Int64.required_field("id"),
                    DataType::Null.nullable_field("gap"),
                ])
                .map(DataType::from)
                .unwrap(),
                false,
            );
            let arrow_schema = schema.into_arrow_schema().unwrap();
            let batch = RecordBatch::try_new(
                arrow_schema,
                vec![
                    Arc::new(arrow_array::Int64Array::from(vec![1, 2])),
                    Arc::new(arrow_array::NullArray::new(2)),
                ],
            )
            .unwrap();
            let mut handle = handle();
            avro::overwrite_arrow_reader(
                &mut handle,
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &AvroOptions::new(),
            )
            .unwrap();
            let read: Vec<RecordBatch> =
                avro::read_batch_reader(&handle, None, &AvroOptions::new())
                    .unwrap()
                    .collect::<Result<_, _>>()
                    .unwrap();
            assert_eq!(read[0].num_rows(), 2);
            assert_eq!(read[0].column(1).logical_null_count(), 2);
        }

        #[test]
        fn an_out_of_range_duration_is_refused_in_both_directions() {
            // Encode: the wire counts are unsigned, so a negative arrow interval
            // cannot be spelled.
            let schema = Field::new(
                "row",
                StructType::from_fields([DataType::interval(yggdryl::TimeUnit::MonthDayNano)
                    .unwrap()
                    .required_field("span")])
                .map(DataType::from)
                .unwrap(),
                false,
            );
            let arrow_schema = schema.into_arrow_schema().unwrap();
            let batch = RecordBatch::try_new(
                arrow_schema,
                vec![Arc::new(arrow_array::IntervalMonthDayNanoArray::from(
                    vec![arrow_buffer::IntervalMonthDayNano::new(-1, 0, 0)],
                ))],
            )
            .unwrap();
            let mut handle = handle();
            let message = avro::overwrite_arrow_reader(
                &mut handle,
                yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                &AvroOptions::new(),
            )
            .unwrap_err()
            .to_string();
            assert!(
                message.contains("non-negative duration months"),
                "{message}"
            );

            // Decode: a wire count above 31 bits cannot fit the arrow interval
            // and is refused, never clamped.
            let mut payload = vec![0xFF, 0xFF, 0xFF, 0xFF];
            payload.extend_from_slice(&[0; 8]);
            let handle = super::handmade_container(
                r#"{"type":"record","name":"row","fields":[
                {"name":"span","type":
                    {"type":"fixed","name":"dur","size":12,"logicalType":"duration"}}
            ]}"#,
                "null",
                &[(1, payload)],
            );
            let message = avro::read_batch_reader(&handle, None, &AvroOptions::new())
                .unwrap()
                .collect::<Result<Vec<RecordBatch>, _>>()
                .unwrap_err()
                .to_string();
            assert!(message.contains("within 31 bits"), "{message}");
        }

        #[test]
        fn logical_columns_round_trip_columnar() {
            let schema = Field::new(
                "row",
                StructType::from_fields([
                    DataType::date32().required_field("day"),
                    DataType::DateTime64 {
                        unit: yggdryl::TimeUnit::Microsecond,
                        timezone: yggdryl::Timezone::UTC,
                    }
                    .nullable_field("at"),
                    DataType::decimal(10, 2).unwrap().required_field("cost"),
                ])
                .map(DataType::from)
                .unwrap(),
                false,
            );
            let arrow_schema = schema.into_arrow_schema().unwrap();
            let batch = RecordBatch::try_new(
                arrow_schema,
                vec![
                    Arc::new(arrow_array::Date32Array::from(vec![19_782, -3_652])),
                    Arc::new(
                        arrow_array::TimestampMicrosecondArray::from(vec![
                            Some(1_700_000_000_000_000),
                            None,
                        ])
                        .with_timezone("UTC"),
                    ),
                    Arc::new(
                        arrow_array::Decimal64Array::from(vec![18_750_i64, -99])
                            .with_precision_and_scale(10, 2)
                            .unwrap(),
                    ),
                ],
            )
            .unwrap();

            let mut handle = handle();
            let options = AvroOptions::new();
            avro::overwrite_arrow_reader(
                &mut handle,
                yggdryl::arrow::batch_reader(batch.schema(), [batch.clone()]),
                &options,
            )
            .unwrap();

            // The container is readable at the Scalar level with typed temporals.
            let container = avro::read_container(&handle).unwrap();
            assert_eq!(
                container.rows[0].get_key_str("day"),
                Some(&yggdryl::Scalar::date32(19_782))
            );
            assert_eq!(
                container.rows[1].get_key_str("cost"),
                Some(
                    &DataType::decimal(10, 2)
                        .unwrap()
                        .scalar(yggdryl::Scalar::decimal128(-99, 2))
                        .unwrap()
                )
            );

            // And columnar reads reproduce the arrays exactly.
            let read: Vec<RecordBatch> = avro::read_batch_reader(&handle, None, &options)
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            assert_eq!(read[0].column(0).as_ref(), batch.column(0).as_ref());
            assert_eq!(read[0].column(1).as_ref(), batch.column(1).as_ref());
            assert_eq!(read[0].column(2).as_ref(), batch.column(2).as_ref());
        }

        /// Every verb that changes what the handle holds without saying what
        /// ends the answers an open session held: the next ask reads the container,
        /// and the session holds what it read again.
        #[test]
        fn every_byte_verb_drops_what_an_open_session_holds() {
            use yggdryl::holder::counted::{Calls, Counted, Group};

            for verb in ["handle_mut", "set_media_type", "pwrite", "truncate"] {
                let (_, template) = batch();
                let mut writer = Avro::new(handle());
                let options = writer.record_options().unwrap();
                writer
                    .overwrite_arrow_reader(
                        yggdryl::arrow::batch_reader(template.schema(), [template]),
                        &options,
                    )
                    .unwrap();
                let counted = Counted::new(writer.into_handle());
                let calls: Arc<Calls> = Arc::clone(counted.calls());
                let mut media = Avro::new(counted);
                media.open().unwrap();

                let asks = |media: &Avro<Counted<Buffer>>| {
                    media.read_origin_field().unwrap();
                    media.row_size().unwrap();
                    media.column_size().unwrap();
                };
                asks(&media);
                calls.reset();
                asks(&media);
                assert_eq!(calls.group(Group::Read), 0, "{verb}: warm before");
                match verb {
                    "handle_mut" => {
                        let _ = media.handle_mut();
                    }
                    "set_media_type" => {
                        let media_type = media.media_type().clone();
                        media.set_media_type(media_type);
                    }
                    "pwrite" => {
                        let first = media.read_range_bytes(0, 1).unwrap();
                        media.pwrite(0, &first).unwrap();
                    }
                    _ => {
                        let size = media.handle().size();
                        media.truncate(size).unwrap();
                    }
                }
                calls.reset();
                asks(&media);
                assert!(
                    calls.group(Group::Read) > 0,
                    "{verb}: the held answers are dropped"
                );
                calls.reset();
                asks(&media);
                assert_eq!(calls.group(Group::Read), 0, "{verb}: and held again");
            }
        }

        /// What a read decodes and what the container states, seen from the
        /// record surface.
        mod origin {
            use arrow_array::RecordBatch;

            use yggdryl::StructType;
            use yggdryl::avro::Avro;
            use yggdryl::holder::Buffer;
            use yggdryl::media::IORecordOptions;
            use yggdryl::{DataType, Field, IOMedia, Scalar};

            use super::super::{handmade_container, put_bytes, put_long};
            use super::{batch, handle};

            /// The names of a root's columns.
            fn names(field: &Field) -> Vec<&str> {
                field.fields().iter().map(Field::name).collect()
            }

            /// A container of two rows whose `value` strings are `ok` and bytes
            /// that are not UTF-8: a column nobody may decode.
            fn garbled() -> Buffer {
                let mut payload = Vec::new();
                put_long(&mut payload, 1);
                put_bytes(&mut payload, b"ok");
                put_long(&mut payload, 2);
                put_bytes(&mut payload, &[0xff]);
                handmade_container(
                    r#"{"type":"record","name":"rows","fields":[
                        {"name":"id","type":"long"},{"name":"value","type":"string"}]}"#,
                    "null",
                    &[(2, payload)],
                )
            }

            /// The two columns of `garbled`, declared in full.
            fn declared() -> Field {
                StructType::from_fields([
                    DataType::Int64.required_field("id"),
                    DataType::utf8().required_field("value"),
                ])
                .map(DataType::from)
                .unwrap()
                .required_field("rows")
            }

            #[test]
            fn a_full_declared_field_with_a_select_decodes_the_selected_columns_alone() {
                let media = Avro::new(garbled());
                let whole = media.record_options().unwrap().with_field(declared());

                // The control: the column of garbage is decoded when it is asked for.
                let failed = match media.read_arrow_reader(&whole) {
                    Err(_) => true,
                    Ok(mut reader) => reader.any(|batch| batch.is_err()),
                };
                assert!(failed, "the whole row decodes");
                let rows = (|| -> yggdryl::Result<Vec<Scalar>> {
                    IOMedia::read_serie(&media, Some(&whole))?
                        .into_stream()?
                        .collect_rows()
                })();
                assert!(rows.is_err(), "and so does the stream");

                // The declared field names both columns and the select keeps one.
                let selected = whole.with_select("id").unwrap();
                let batches: Vec<RecordBatch> = media
                    .read_arrow_reader(&selected)
                    .unwrap()
                    .collect::<Result<_, _>>()
                    .expect("the selected column decodes");
                assert_eq!(batches.iter().map(RecordBatch::num_rows).sum::<usize>(), 2);
                assert!(batches.iter().all(|batch| batch.num_columns() == 1));
                let rows = IOMedia::read_serie(&media, Some(&selected))
                    .unwrap()
                    .into_stream()
                    .unwrap()
                    .collect_rows()
                    .unwrap();
                assert_eq!(
                    rows,
                    [1_i64, 2].map(|id| Scalar::from_sequence([Scalar::from(id)]))
                );
            }

            #[test]
            fn one_schema_answer_under_a_select_whether_the_field_is_declared_or_stored() {
                let (field, batch) = batch();
                let mut writer = Avro::new(handle()).with_name("trades");
                let options = writer.record_options().unwrap();
                writer
                    .overwrite_arrow_reader(
                        yggdryl::arrow::batch_reader(batch.schema(), [batch]),
                        &options,
                    )
                    .unwrap();
                let media = Avro::new(writer.into_handle()).with_name("trades");

                let selected = media.record_options().unwrap().with_select("id").unwrap();
                let stored = media.read_arrow_field(&selected).unwrap();
                assert_eq!(names(&stored), ["id"]);
                let declared = media
                    .read_arrow_field(&selected.clone().with_field(field.clone()))
                    .unwrap();
                assert_eq!(names(&declared), ["id"]);
                assert_eq!(stored.fields()[0].dtype(), declared.fields()[0].dtype());

                // It is the shape the read publishes, too.
                let read = media
                    .read_arrow_reader(&selected.with_field(field))
                    .unwrap()
                    .schema();
                assert_eq!(
                    names(&Field::from_arrow_schema("trades", read.as_ref()).unwrap()),
                    ["id"]
                );
            }

            #[test]
            fn the_origin_is_what_the_container_states_whatever_the_options_declare_or_select() {
                let media = Avro::new(garbled()).with_options(
                    yggdryl::avro::AvroOptions::new()
                        .with_field(
                            StructType::from_fields([DataType::Int64.required_field("id")])
                                .map(DataType::from)
                                .unwrap()
                                .required_field("rows"),
                        )
                        .with_select("id")
                        .unwrap(),
                );
                let origin = media.read_origin_field().unwrap().expect("a stated shape");
                assert_eq!(names(&origin), ["id", "value"]);
                let published = media
                    .read_arrow_field(&media.record_options().unwrap())
                    .unwrap();
                assert_eq!(names(&published), ["id"]);
            }

            #[test]
            fn an_empty_container_states_no_origin_and_no_schema_unless_one_is_declared() {
                let media = Avro::new(Buffer::new());
                assert!(media.read_origin_field().unwrap().is_none());
                let options = media.record_options().unwrap();
                let error = media.read_arrow_field(&options).unwrap_err().to_string();
                assert!(error.contains("$.field"), "{error}");
                assert!(error.contains("no schema"), "{error}");
                assert_eq!(
                    media
                        .read_arrow_field(&options.with_field(declared()))
                        .unwrap(),
                    declared()
                );
            }
        }

        /// What a closed container's metadata cache serves under the options'
        /// `cache_ttl`, with the clock the cache doors read installed by hand, and
        /// what every write and every byte verb does to it.
        #[cfg(feature = "internals")]
        mod ttl {
            use std::sync::Arc;
            use std::time::{Duration, Instant};

            use yggdryl::avro::{Avro, AvroOptions};
            use yggdryl::holder::Buffer;
            use yggdryl::holder::counted::{Calls, Counted, Group};
            use yggdryl::internals::media_cache::with_clock;
            use yggdryl::media::IORecordOptions;
            use yggdryl::{Field, IOBase, IOMedia};

            use super::{Shared, batch, handle};

            /// The names of a root's columns.
            fn names(field: &Field) -> Vec<&str> {
                field.fields().iter().map(Field::name).collect()
            }

            /// A container of `copies` copies of the three-row batch.
            fn encoded(copies: usize) -> Buffer {
                let (_, template) = batch();
                let schema = template.schema();
                let batches = std::iter::repeat_n(template, copies);
                let mut media = Avro::new(handle());
                let options = media.record_options().unwrap();
                media
                    .overwrite_arrow_reader(yggdryl::arrow::batch_reader(schema, batches), &options)
                    .unwrap();
                media.into_handle()
            }

            /// A container of `copies` batches over a counted handle, read under a
            /// `ttl` of milliseconds.
            fn counted(copies: usize, ttl: u64) -> (Avro<Counted<Buffer>>, Arc<Calls>) {
                let counted = Counted::new(encoded(copies));
                let calls = Arc::clone(counted.calls());
                let media = Avro::new(counted).with_options(AvroOptions::new().with_cache_ttl(ttl));
                (media, calls)
            }

            /// The three answers the cache serves: the origin, the rows, the columns.
            fn ask<H: IOBase>(media: &Avro<H>) -> (Option<Field>, u64, usize) {
                (
                    media.read_origin_field().unwrap(),
                    media.row_size().unwrap(),
                    media.column_size().unwrap(),
                )
            }

            /// The bytes read from the store while `ask` runs.
            fn reads(calls: &Calls, ask: impl FnOnce()) -> u64 {
                calls.reset();
                ask();
                calls.group(Group::Read)
            }

            #[test]
            fn a_warm_closed_read_under_a_ttl_costs_no_store_call_until_the_entry_is_as_old_as_the_ttl()
             {
                let (media, calls) = counted(1, 1_000);
                let options = media.record_options().unwrap();
                let t0 = Instant::now();
                let at = |milliseconds: u64| t0 + Duration::from_millis(milliseconds);

                let (origin, rows, columns) = with_clock(t0, || ask(&media));
                assert_eq!((rows, columns), (3, 4));
                assert_eq!(
                    names(origin.as_ref().expect("a stated shape")),
                    ["id", "symbol", "price", "legs"]
                );
                assert!(
                    calls.group(Group::Read) > 0,
                    "the first ask reads the header"
                );

                let cost = reads(&calls, || {
                    let warm = with_clock(at(999), || ask(&media));
                    assert_eq!(warm.0, origin);
                    assert_eq!((warm.1, warm.2), (rows, columns));
                    let field = with_clock(at(999), || media.read_arrow_field(&options).unwrap());
                    assert_eq!(field.field_len(), 4);
                });
                assert_eq!(cost, 0, "a warm closed read is free");

                assert!(reads(&calls, || drop(with_clock(at(1_000), || ask(&media)))) > 0);
                assert_eq!(
                    reads(&calls, || drop(with_clock(at(1_999), || ask(&media)))),
                    0
                );
                assert!(reads(&calls, || drop(with_clock(at(2_000), || ask(&media)))) > 0);
            }

            #[test]
            fn a_realtime_closed_handle_reads_afresh_on_every_ask() {
                let (media, calls) = counted(1, 0);
                let first = reads(&calls, || drop(ask(&media)));
                let second = reads(&calls, || drop(ask(&media)));
                assert!(first > 0, "a closed realtime ask reads the container");
                assert_eq!(second, first, "and what it read is not served again");
            }

            #[test]
            fn an_open_handle_serves_whatever_the_ttl_says_until_it_closes() {
                let t0 = Instant::now();
                for ttl in [0, 1_000] {
                    let (mut media, calls) = counted(1, ttl);
                    media.open().unwrap();
                    with_clock(t0, || drop(ask(&media)));
                    let hour = t0 + Duration::from_secs(3_600);
                    assert_eq!(
                        reads(&calls, || drop(with_clock(hour, || ask(&media)))),
                        0,
                        "an open session serves, ttl {ttl}"
                    );
                    media.close().unwrap();
                    assert!(
                        reads(&calls, || drop(with_clock(hour, || ask(&media)))) > 0,
                        "closing dropped the entry, ttl {ttl}"
                    );
                }
            }

            #[test]
            fn an_out_of_band_write_is_seen_after_the_ttl_and_not_before() {
                let shared = Shared::new(encoded(1));
                let mut external = shared.clone();
                let media = Avro::new(Counted::new(shared))
                    .with_options(AvroOptions::new().with_cache_ttl(1_000_u64));
                let t0 = Instant::now();
                let at = |milliseconds: u64| t0 + Duration::from_millis(milliseconds);
                assert_eq!(with_clock(t0, || media.row_size().unwrap()), 3);

                external.write_all_bytes(encoded(2).as_slice()).unwrap();
                assert_eq!(
                    with_clock(at(999), || media.row_size().unwrap()),
                    3,
                    "the entry is younger than the TTL"
                );
                assert_eq!(with_clock(at(1_000), || media.row_size().unwrap()), 6);
            }

            #[test]
            fn an_overwrite_answers_the_origin_and_the_rows_with_zero_reads() {
                let (mut media, calls) = counted(1, 1_000);
                let options = media.record_options().unwrap();
                let (_, template) = batch();
                let t0 = Instant::now();

                with_clock(t0, || {
                    media
                        .overwrite_arrow_reader(
                            yggdryl::arrow::batch_reader(
                                template.schema(),
                                [template.clone(), template.clone()],
                            ),
                            &options,
                        )
                        .unwrap();
                });
                let mut answers = None;
                let cost = reads(&calls, || {
                    answers = Some(with_clock(t0 + Duration::from_millis(500), || ask(&media)));
                });
                let (origin, rows, columns) = answers.unwrap();
                assert_eq!(cost, 0, "the write said what it published");
                assert_eq!((rows, columns), (6, 4));
                assert_eq!(
                    names(&origin.expect("the published root")),
                    ["id", "symbol", "price", "legs"]
                );
            }

            #[test]
            fn an_overwrite_of_an_open_handle_answers_with_zero_reads_whatever_the_ttl() {
                let (mut media, calls) = counted(1, 0);
                media.open().unwrap();
                let options = media.record_options().unwrap();
                let (_, template) = batch();
                media
                    .overwrite_arrow_reader(
                        yggdryl::arrow::batch_reader(
                            template.schema(),
                            [template.clone(), template.clone()],
                        ),
                        &options,
                    )
                    .unwrap();

                let mut answers = None;
                let cost = reads(&calls, || answers = Some(ask(&media)));
                let (origin, rows, columns) = answers.unwrap();
                assert_eq!(cost, 0, "the open session kept what the write published");
                assert_eq!((rows, columns), (6, 4));
                assert_eq!(
                    names(&origin.expect("the published root")),
                    ["id", "symbol", "price", "legs"]
                );
                assert!(media.opened());
            }

            #[test]
            fn a_closed_realtime_write_keeps_nothing_it_could_serve() {
                let (mut media, calls) = counted(1, 0);
                let options = media.record_options().unwrap();
                let (_, template) = batch();
                media
                    .overwrite_arrow_reader(
                        yggdryl::arrow::batch_reader(
                            template.schema(),
                            [template.clone(), template],
                        ),
                        &options,
                    )
                    .unwrap();
                assert!(reads(&calls, || assert_eq!(media.row_size().unwrap(), 6)) > 0);
                assert!(!media.opened(), "a write starts no session");
            }

            #[test]
            fn an_append_adds_the_rows_it_wrote_to_the_rows_it_holds() {
                let (mut media, calls) = counted(1, 1_000);
                let options = media.record_options().unwrap();
                let (_, template) = batch();
                let t0 = Instant::now();
                assert_eq!(with_clock(t0, || media.row_size().unwrap()), 3);

                with_clock(t0, || {
                    media
                        .append_arrow_reader(
                            yggdryl::arrow::batch_reader(template.schema(), [template]),
                            &options,
                        )
                        .unwrap();
                });
                let mut rows = 0;
                let cost = reads(&calls, || {
                    rows = with_clock(t0 + Duration::from_millis(1), || media.row_size().unwrap());
                });
                assert_eq!(rows, 6, "three held rows and three appended");
                assert_eq!(cost, 0, "the append said what the container now holds");
            }

            #[test]
            fn clear_answers_no_origin_and_zero_rows_with_zero_reads() {
                let (mut media, calls) = counted(1, 1_000);
                let t0 = Instant::now();
                with_clock(t0, || {
                    assert_eq!(ask(&media).1, 3);
                    media.clear().unwrap();
                });
                let mut answers = None;
                let cost = reads(&calls, || {
                    answers = Some(with_clock(t0 + Duration::from_millis(10), || ask(&media)));
                });
                assert_eq!(cost, 0);
                let (origin, rows, columns) = answers.unwrap();
                assert!(origin.is_none(), "an emptied container states no shape");
                assert_eq!((rows, columns), (0, 0));
            }

            /// Every verb that changes what the handle holds without saying what
            /// leaves the next ask to read it - `set_media_type` among them, which
            /// no cache dropped before.
            #[test]
            fn every_byte_verb_drops_the_entry() {
                let t0 = Instant::now();
                for verb in ["handle_mut", "set_media_type", "pwrite", "truncate"] {
                    let (mut media, calls) = counted(1, 1_000);
                    with_clock(t0, || drop(ask(&media)));
                    assert_eq!(
                        reads(&calls, || drop(with_clock(t0, || ask(&media)))),
                        0,
                        "{verb}: warm before"
                    );
                    match verb {
                        "handle_mut" => {
                            let _ = media.handle_mut();
                        }
                        "set_media_type" => {
                            let media_type = media.media_type().clone();
                            media.set_media_type(media_type);
                        }
                        "pwrite" => {
                            let first = media.read_range_bytes(0, 1).unwrap();
                            media.pwrite(0, &first).unwrap();
                        }
                        _ => {
                            let size = media.handle().size();
                            media.truncate(size).unwrap();
                        }
                    }
                    assert!(
                        reads(&calls, || drop(with_clock(t0, || ask(&media)))) > 0,
                        "{verb}: the entry is dropped"
                    );
                    assert_eq!(
                        reads(&calls, || drop(with_clock(t0, || ask(&media)))),
                        0,
                        "{verb}: and read again"
                    );
                }
            }

            #[test]
            fn a_merge_and_a_removal_leave_no_stale_answer() {
                let t0 = Instant::now();
                let (mut media, _) = counted(1, 1_000);
                with_clock(t0, || drop(ask(&media)));
                media
                    .options_mut()
                    .set_merge_by(yggdryl::expression::Selector::from_columns(["id"]));
                let options = media.record_options().unwrap();
                let (_, template) = batch();
                with_clock(t0, || {
                    media
                        .merge_arrow_reader(
                            yggdryl::arrow::batch_reader(template.schema(), [template]),
                            &options,
                        )
                        .unwrap();
                });
                // The same three ids: the merge replaced every row and added none.
                assert_eq!(with_clock(t0, || media.row_size().unwrap()), 3);

                media.remove(false).unwrap();
                assert_eq!(with_clock(t0, || media.row_size().unwrap()), 0);
                assert!(with_clock(t0, || media.read_origin_field().unwrap()).is_none());
            }
        }
    }

    mod limits {

        use std::sync::Arc;
        use yggdryl::StructType;

        use arrow_array::RecordBatchReader;

        use yggdryl::IOMedia;
        use yggdryl::holder::Buffer;
        use yggdryl::media::{IORecordOptions, RecordOptions};
        use yggdryl::{DataType, Field, Url};

        /// A struct field is the schema of the batches it describes.
        fn schema() -> Field {
            StructType::from_fields([DataType::Int64.required_field("id")])
                .map(DataType::from)
                .unwrap()
                .required_field("row")
        }

        /// A reader over one batch of `ids`.
        fn reader(ids: Vec<i64>) -> yggdryl::arrow::BatchReader {
            let batch = arrow_array::RecordBatch::try_new(
                schema().into_arrow_schema().unwrap(),
                vec![Arc::new(arrow_array::Int64Array::from(ids))],
            )
            .unwrap();
            yggdryl::arrow::batch_reader(batch.schema(), [batch])
        }

        fn handle() -> Buffer {
            Buffer::new()
                .with_media_type(Url::from_str("file:///limited.avro").unwrap().media_type())
        }

        /// The total rows a handle yields under `options`.
        fn rows(handle: &Buffer, options: &RecordOptions) -> usize {
            handle
                .read_arrow_reader(options)
                .unwrap()
                .map(|batch| batch.unwrap().num_rows())
                .sum()
        }

        #[test]
        fn a_zero_limit_reads_the_declared_schema_and_no_batches() {
            let mut handle = handle();
            let options = handle.record_options().unwrap().with_field(schema());
            handle
                .overwrite_arrow_reader(reader(vec![1, 2]), &options)
                .unwrap();

            let mut limited = handle
                .read_arrow_reader(&options.with_max_row_size(0))
                .unwrap();
            // The schema is asserted, not only the emptiness: `Some(0)` is a
            // valid ask that still says what the rows would have been.
            assert_eq!(limited.schema(), schema().into_arrow_schema().unwrap());
            assert!(limited.next().is_none());
        }

        #[test]
        fn a_limited_write_truncates_what_the_caller_offered() {
            let mut handle = handle();
            let options = handle.record_options().unwrap().with_field(schema());

            handle
                .overwrite_arrow_reader(reader(vec![1, 2]), &options.clone().with_max_row_size(1))
                .unwrap();
            assert_eq!(rows(&handle, &options), 1);

            // An append is a write, so the same bound truncates it the same way.
            handle
                .append_arrow_reader(reader(vec![3, 4]), &options.clone().with_max_row_size(1))
                .unwrap();
            assert_eq!(rows(&handle, &options), 2);
        }
    }

    /// Containers large enough to decode and encode on several threads.
    mod parallel {
        use std::sync::Arc;

        use arrow_array::cast::AsArray;
        use arrow_array::types::{Float64Type, Int64Type};
        use arrow_array::{
            Array, ArrayRef, BinaryArray, BooleanArray, Date32Array, Float32Array, Float64Array,
            Int32Array, Int64Array, RecordBatch, StringArray, TimestampMicrosecondArray,
        };
        use arrow_schema::{DataType as ArrowType, Field as ArrowField, Schema, TimeUnit};

        use yggdryl::avro::{self, AvroOptions};
        use yggdryl::holder::Buffer;
        use yggdryl::{DataType, Field, StructType};

        use super::{buffer, handmade_container, put_long};

        const ROWS: usize = 150_000;

        /// Trades whose null codec keeps a container above a megabyte.
        fn trades(rows: usize) -> RecordBatch {
            let ids: Int64Array = (0..rows as i64).collect();
            let symbols: StringArray = (0..rows)
                .map(|row| (row % 7 != 0).then(|| format!("SYM{}", row % 97)))
                .collect();
            let prices: Float64Array = (0..rows).map(|row| row as f64 * 0.25).collect();
            RecordBatch::try_new(
                Arc::new(Schema::new(vec![
                    ArrowField::new("id", ArrowType::Int64, false),
                    ArrowField::new("symbol", ArrowType::Utf8, true),
                    ArrowField::new("price", ArrowType::Float64, false),
                ])),
                vec![Arc::new(ids), Arc::new(symbols), Arc::new(prices)],
            )
            .unwrap()
        }

        /// Options that decode and encode on four threads whatever the host
        /// offers, where the `internals` hook can pin them.
        fn threaded(options: AvroOptions) -> AvroOptions {
            #[cfg(feature = "internals")]
            let options = yggdryl::internals::avro_batch::with_threads(options, 4);
            options
        }

        fn written(batch: &RecordBatch) -> Buffer {
            let mut handle = buffer();
            avro::overwrite_arrow_reader(
                &mut handle,
                yggdryl::arrow::batch_reader(batch.schema(), [batch.clone()]),
                &threaded(AvroOptions::new().with_codec("null")),
            )
            .unwrap();
            handle
        }

        fn read(handle: &Buffer, field: Option<&Field>, options: &AvroOptions) -> Vec<RecordBatch> {
            avro::read_batch_reader(handle, field, &threaded(options.clone()))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        }

        /// The row count of every block a container holds.
        fn block_counts(handle: &Buffer) -> Vec<u64> {
            let mut blocks = avro::read_blocks(handle).unwrap();
            let mut counts = Vec::new();
            while let Some(block) = blocks.next_block().unwrap() {
                counts.push(block.count());
            }
            counts
        }

        #[test]
        fn a_million_booleans_are_cut_into_blocks_a_reader_accepts() {
            let rows = 1_100_000;
            let flags: BooleanArray = (0..rows).map(|row| Some(row % 3 == 0)).collect();
            let batch = RecordBatch::try_new(
                Arc::new(Schema::new(vec![ArrowField::new(
                    "flag",
                    ArrowType::Boolean,
                    false,
                )])),
                vec![Arc::new(flags) as ArrayRef],
            )
            .unwrap();
            let handle = written(&batch);
            let counts = block_counts(&handle);
            assert!(counts.iter().all(|count| *count <= 1_000_000), "{counts:?}");
            let read = read(&handle, None, &AvroOptions::new());
            let whole = arrow_select::concat::concat_batches(&read[0].schema(), &read).unwrap();
            assert_eq!(whole.column(0).as_ref(), batch.column(0).as_ref());
        }

        #[test]
        fn a_sliced_batch_is_cut_as_its_own_rows_not_its_parents() {
            let batch = trades(ROWS);
            let owned = block_counts(&written(&batch)).len();
            // The same rows arriving as zero-copy slices of one batch.
            let mut handle = buffer();
            let slices: Vec<RecordBatch> = (0..ROWS)
                .step_by(10_000)
                .map(|start| batch.slice(start, 10_000.min(ROWS - start)))
                .collect();
            avro::overwrite_arrow_reader(
                &mut handle,
                yggdryl::arrow::batch_reader(batch.schema(), slices),
                &threaded(AvroOptions::new().with_codec("null")),
            )
            .unwrap();
            let sliced = block_counts(&handle).len();
            // A slice never splits into more blocks than its rows need: each
            // 10,000-row slice is at most one block.
            assert!(
                sliced <= ROWS.div_ceil(10_000).max(owned),
                "{sliced} blocks vs {owned}"
            );
            let read = read(&handle, None, &AvroOptions::new());
            let whole = arrow_select::concat::concat_batches(&read[0].schema(), &read).unwrap();
            assert_eq!(whole.num_rows(), ROWS);
        }

        #[test]
        fn a_sliced_list_column_is_cut_by_the_child_rows_its_slice_spans() {
            use arrow_array::builder::{Int64Builder, ListBuilder};

            let rows = 150_000;
            let mut legs = ListBuilder::new(Int64Builder::new());
            for row in 0..rows {
                for leg in 0..4 {
                    legs.values().append_value(row as i64 * 4 + leg);
                }
                legs.append(true);
            }
            let legs = legs.finish();
            let batch = RecordBatch::try_new(
                Arc::new(Schema::new(vec![ArrowField::new(
                    "legs",
                    legs.data_type().clone(),
                    false,
                )])),
                vec![Arc::new(legs) as ArrayRef],
            )
            .unwrap();
            let slices: Vec<RecordBatch> = (0..rows)
                .step_by(10_000)
                .map(|start| batch.slice(start, 10_000))
                .collect();
            let mut handle = buffer();
            avro::overwrite_arrow_reader(
                &mut handle,
                yggdryl::arrow::batch_reader(batch.schema(), slices),
                &threaded(AvroOptions::new().with_codec("null")),
            )
            .unwrap();
            // A 10,000-row slice spans 40,000 child values - far under a
            // block - so each slice is one block, not a fraction of one.
            let counts = block_counts(&handle);
            assert_eq!(counts.len(), rows / 10_000, "{counts:?}");
            let read = read(&handle, None, &AvroOptions::new());
            let whole = arrow_select::concat::concat_batches(&read[0].schema(), &read).unwrap();
            assert_eq!(whole.num_rows(), rows);
        }

        #[test]
        fn a_large_write_is_cut_into_blocks_that_read_back_in_order() {
            let batch = trades(ROWS);
            let handle = written(&batch);

            // One incoming batch, several blocks: what lets a reader split it.
            let mut blocks = avro::read_blocks(&handle).unwrap();
            let mut counts = Vec::new();
            while let Some(block) = blocks.next_block().unwrap() {
                counts.push(block.count());
            }
            assert!(counts.len() > 1, "{counts:?}");
            assert_eq!(counts.iter().sum::<u64>(), ROWS as u64);

            let read = read(&handle, None, &AvroOptions::new());
            let whole = arrow_select::concat::concat_batches(&read[0].schema(), &read).unwrap();
            assert_eq!(whole.num_rows(), ROWS);
            for column in 0..batch.num_columns() {
                assert_eq!(whole.column(column).as_ref(), batch.column(column).as_ref());
            }
        }

        #[test]
        fn a_batch_never_holds_more_rows_than_asked_for() {
            let handle = written(&trades(ROWS));
            let mut options = AvroOptions::new();
            options.batch_row_size = Some(10_000);
            let read = read(&handle, None, &options);
            assert!(read.iter().all(|batch| batch.num_rows() <= 10_000));
            let ids: Vec<i64> = read
                .iter()
                .flat_map(|batch| {
                    batch
                        .column(0)
                        .as_primitive::<Int64Type>()
                        .values()
                        .to_vec()
                })
                .collect();
            assert_eq!(ids, (0..ROWS as i64).collect::<Vec<_>>());
        }

        #[test]
        fn a_projection_skips_on_every_thread() {
            let batch = trades(ROWS);
            let handle = written(&batch);
            let field = StructType::from_fields([DataType::Float64.required_field("price")])
                .map(DataType::from)
                .unwrap()
                .required_field("trades");
            let read = read(&handle, Some(&field), &AvroOptions::new());
            assert!(read.iter().all(|batch| batch.num_columns() == 1));
            let prices: Vec<f64> = read
                .iter()
                .flat_map(|batch| {
                    batch
                        .column(0)
                        .as_primitive::<Float64Type>()
                        .values()
                        .to_vec()
                })
                .collect();
            assert_eq!(
                prices,
                batch
                    .column(2)
                    .as_primitive::<Float64Type>()
                    .values()
                    .to_vec()
            );
        }

        /// Four blocks of one `long` column; `declared` is block three's count.
        fn four_blocks(declared: i64) -> Buffer {
            let per_block = 120_000_i64;
            let blocks: Vec<(i64, Vec<u8>)> = (0..4)
                .map(|block| {
                    let mut payload = Vec::new();
                    for row in 0..per_block {
                        put_long(&mut payload, block * per_block + row);
                    }
                    (if block == 2 { declared } else { per_block }, payload)
                })
                .collect();
            handmade_container(
                r#"{"type":"record","name":"r","fields":[{"name":"id","type":"long"}]}"#,
                "null",
                &blocks,
            )
        }

        #[test]
        fn a_block_short_of_its_rows_fails_the_read_after_the_rows_before_it() {
            let handle = four_blocks(120_001);
            let mut ids = Vec::new();
            let mut failure = None;
            for batch in avro::read_batch_reader(&handle, None, &AvroOptions::new()).unwrap() {
                match batch {
                    Ok(batch) => {
                        ids.extend_from_slice(batch.column(0).as_primitive::<Int64Type>().values())
                    }
                    Err(error) => {
                        failure = Some(error.to_string());
                        break;
                    }
                }
            }
            let failure = failure.expect("a block that runs out of rows is an error");
            assert!(failure.contains("expected"), "{failure}");
            // What came first is every row in front of the fault, in order, and
            // nothing from the block behind it.
            assert!(ids.len() >= 240_000 && ids.len() < 360_000, "{}", ids.len());
            assert_eq!(ids, (0..ids.len() as i64).collect::<Vec<_>>());
        }

        #[test]
        fn a_block_with_rows_left_over_fails_the_read() {
            let handle = four_blocks(119_999);
            let failure = avro::read_batch_reader(&handle, None, &AvroOptions::new())
                .unwrap()
                .find_map(Result::err)
                .expect("a block with bytes after its declared rows is an error")
                .to_string();
            assert!(failure.contains("declared rows"), "{failure}");
        }

        #[test]
        fn every_flat_type_round_trips_through_the_resolved_writers() {
            let rows = 2_000;
            let nullable = |row: usize| !row.is_multiple_of(5);
            let columns: Vec<(ArrowField, ArrayRef)> = vec![
                (
                    ArrowField::new("int", ArrowType::Int32, true),
                    Arc::new(
                        (0..rows)
                            .map(|row| nullable(row).then_some(row as i32 - 1_000))
                            .collect::<Int32Array>(),
                    ),
                ),
                (
                    ArrowField::new("long", ArrowType::Int64, false),
                    Arc::new(
                        (0..rows)
                            .map(|row| row as i64 * -7_919)
                            .collect::<Int64Array>(),
                    ),
                ),
                (
                    ArrowField::new("float", ArrowType::Float32, true),
                    Arc::new(
                        (0..rows)
                            .map(|row| nullable(row).then_some(row as f32 / 3.0))
                            .collect::<Float32Array>(),
                    ),
                ),
                (
                    ArrowField::new("double", ArrowType::Float64, false),
                    Arc::new(
                        (0..rows)
                            .map(|row| row as f64 / 7.0)
                            .collect::<Float64Array>(),
                    ),
                ),
                (
                    ArrowField::new("flag", ArrowType::Boolean, true),
                    Arc::new(
                        (0..rows)
                            .map(|row| nullable(row).then_some(row % 2 == 0))
                            .collect::<BooleanArray>(),
                    ),
                ),
                (
                    ArrowField::new("text", ArrowType::Utf8, true),
                    Arc::new(
                        (0..rows)
                            .map(|row| nullable(row).then(|| "é".repeat(row % 4)))
                            .collect::<StringArray>(),
                    ),
                ),
                (
                    ArrowField::new("blob", ArrowType::Binary, true),
                    Arc::new(
                        (0..rows)
                            .map(|row| nullable(row).then(|| vec![row as u8; row % 3]))
                            .collect::<BinaryArray>(),
                    ),
                ),
                (
                    ArrowField::new("day", ArrowType::Date32, true),
                    Arc::new(
                        (0..rows)
                            .map(|row| nullable(row).then_some(19_000 + row as i32))
                            .collect::<Date32Array>(),
                    ),
                ),
                (
                    ArrowField::new(
                        "at",
                        ArrowType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
                        true,
                    ),
                    Arc::new(
                        (0..rows)
                            .map(|row| nullable(row).then_some(1_700_000_000_000_000 + row as i64))
                            .collect::<TimestampMicrosecondArray>()
                            .with_timezone("UTC"),
                    ),
                ),
            ];
            let (fields, arrays): (Vec<_>, Vec<_>) = columns.into_iter().unzip();
            let batch = RecordBatch::try_new(Arc::new(Schema::new(fields)), arrays).unwrap();
            for codec in ["null", "deflate"] {
                let mut handle = buffer();
                avro::overwrite_arrow_reader(
                    &mut handle,
                    yggdryl::arrow::batch_reader(batch.schema(), [batch.clone()]),
                    &AvroOptions::new().with_codec(codec),
                )
                .unwrap();
                let read = read(&handle, None, &AvroOptions::new());
                let whole = arrow_select::concat::concat_batches(&read[0].schema(), &read).unwrap();
                for (index, expected) in batch.columns().iter().enumerate() {
                    let actual = whole.column(index);
                    assert_eq!(actual.len(), expected.len(), "{codec} column {index}");
                    let actual = arrow_cast::cast(actual, expected.data_type()).unwrap();
                    assert_eq!(actual.as_ref(), expected.as_ref(), "{codec} column {index}");
                }
            }
        }
    }

    /// String columns validate their UTF-8 once per batch.
    mod strings {
        use arrow_array::cast::AsArray;

        use yggdryl::avro::{self, AvroOptions};

        use super::{handmade_container, put_bytes};

        const SCHEMA: &str =
            r#"{"type":"record","name":"r","fields":[{"name":"s","type":"string"}]}"#;

        #[test]
        fn multibyte_text_reads_back_whole() {
            let mut payload = Vec::new();
            for text in ["", "é", "日本語", "a\u{1F600}b"] {
                put_bytes(&mut payload, text.as_bytes());
            }
            let handle = handmade_container(SCHEMA, "null", &[(4, payload)]);
            let read: Vec<_> = avro::read_batch_reader(&handle, None, &AvroOptions::new())
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap();
            let texts: Vec<&str> = read[0]
                .column(0)
                .as_string::<i32>()
                .iter()
                .flatten()
                .collect();
            assert_eq!(texts, ["", "é", "日本語", "a\u{1F600}b"]);
        }

        #[test]
        fn invalid_utf8_in_a_string_is_an_error() {
            let mut payload = Vec::new();
            put_bytes(&mut payload, b"fine");
            put_bytes(&mut payload, &[0x66, 0xff, 0x6f]);
            let handle = handmade_container(SCHEMA, "null", &[(2, payload)]);
            let failure = avro::read_batch_reader(&handle, None, &AvroOptions::new())
                .unwrap()
                .find_map(Result::err)
                .expect("a string that is not UTF-8 is an error")
                .to_string();
            assert!(failure.contains("UTF-8"), "{failure}");
        }
    }
}
