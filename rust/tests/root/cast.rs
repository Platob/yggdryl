//! `rust/src/cast.rs`: the cast pairs the engine covers, and the plan that is
//! compiled once and answers every batch of its schema.

use yggdryl::{DataType, Field, StructType};

/// The non-null Struct root a record's rows live under.
fn root(fields: impl IntoIterator<Item = Field>) -> Field {
    StructType::from_fields(fields)
        .map(DataType::from)
        .expect("the root datatype is valid")
        .required_field("row")
}

mod coverage {
    use std::sync::Arc;

    use arrow_array::builder::{Int32Builder, ListBuilder, MapBuilder, StringBuilder};
    use arrow_array::{
        Array, ArrayRef, BinaryViewArray, DictionaryArray, DurationMillisecondArray,
        DurationSecondArray, FixedSizeListArray, Int32Array, Int64Array, ListArray, StringArray,
        StringViewArray, StructArray, Time32SecondArray, Time64MicrosecondArray,
        Time64NanosecondArray, TimestampSecondArray,
    };

    use yggdryl::{ArrowCastOptions, DataType, Field, Serie, TimeUnit, Timezone};

    fn cast(field: &Field, array: ArrayRef) -> yggdryl::arrow::Result<ArrayRef> {
        cast_with(field, array, ArrowCastOptions::new().with_safe(false))
    }

    fn cast_with(
        field: &Field,
        array: ArrayRef,
        options: ArrowCastOptions,
    ) -> yggdryl::arrow::Result<ArrayRef> {
        Ok(Serie::from_arrow_array(Some(field), array, options)?.require_arrow_array()?)
    }

    #[test]
    fn view_layouts_cast_in_and_out_of_their_plain_spellings() {
        // Utf8View -> Utf8 and back, BinaryView -> Binary: the view layouts are
        // first-class datatypes, not lossy coercions.
        let view: ArrayRef = Arc::new(StringViewArray::from(vec![Some("alpha"), None]));
        let plain = cast(&DataType::utf8().nullable_field("text"), Arc::clone(&view)).unwrap();
        assert_eq!(plain.data_type(), &arrow_schema::DataType::Utf8);

        let back = cast(&DataType::utf8_view().nullable_field("text"), plain).unwrap();
        assert_eq!(back.data_type(), &arrow_schema::DataType::Utf8View);

        let bytes: ArrayRef = Arc::new(BinaryViewArray::from(vec![Some(b"ab".as_slice()), None]));
        let plain = cast(&DataType::binary().nullable_field("raw"), bytes).unwrap();
        assert_eq!(plain.data_type(), &arrow_schema::DataType::Binary);
    }

    #[test]
    fn dictionaries_encode_and_decode_scalars() {
        // Utf8 -> Dictionary(Int32, Utf8): encoding a plain column.
        let plain: ArrayRef = Arc::new(StringArray::from(vec!["a", "b", "a"]));
        let target = DataType::from_str("dictionary<int32, utf8>")
            .unwrap()
            .nullable_field("tag");
        let encoded = cast(&target, plain).unwrap();
        let dictionary = encoded
            .as_any()
            .downcast_ref::<DictionaryArray<arrow_array::types::Int32Type>>()
            .expect("a dictionary");
        assert_eq!(dictionary.len(), 3);

        // And back out: Dictionary -> Int64 widens the decoded values.
        let keys = Int32Array::from(vec![0, 1, 0]);
        let values: ArrayRef = Arc::new(Int32Array::from(vec![10, 20]));
        let source: ArrayRef = Arc::new(
            DictionaryArray::<arrow_array::types::Int32Type>::try_new(keys, values).unwrap(),
        );
        let decoded = cast(&DataType::Int64.nullable_field("count"), source).unwrap();
        let decoded = decoded.as_any().downcast_ref::<Int64Array>().unwrap();
        assert_eq!(decoded.values(), &[10, 20, 10]);
    }

    #[test]
    fn serie_wrappers_change_layout_and_cast_their_children() {
        let mut builder = ListBuilder::new(Int32Builder::new());
        builder.values().append_value(1);
        builder.values().append_value(2);
        builder.append(true);
        builder.values().append_value(3);
        builder.append(true);
        let list: ArrayRef = Arc::new(builder.finish());

        // List<Int32> -> LargeList<Int64>: the offset width and the child change.
        let large = cast(
            &DataType::from_str("large_serie<int64>")
                .unwrap()
                .nullable_field("xs"),
            Arc::clone(&list),
        )
        .unwrap();
        assert!(matches!(
            large.data_type(),
            arrow_schema::DataType::LargeList(_)
        ));

        // FixedSizeList<Int32, 2> -> List<Int32>: a size becomes offsets.
        let fixed: ArrayRef = Arc::new(FixedSizeListArray::from_iter_primitive::<
            arrow_array::types::Int32Type,
            _,
            _,
        >(
            vec![Some(vec![Some(1), Some(2)]), Some(vec![Some(3), Some(4)])],
            2,
        ));
        let unsized_list = cast(
            &DataType::from_str("serie<int32>")
                .unwrap()
                .nullable_field("xs"),
            fixed,
        )
        .unwrap();
        assert!(matches!(
            unsized_list.data_type(),
            arrow_schema::DataType::List(_)
        ));
    }

    #[test]
    fn structs_reconcile_by_name_inside_a_serie() {
        // List<Struct{id, name}> -> List<Struct{ID: int64}>: the dedicated arms
        // recurse, select case-insensitively, and drop the extra column.
        let ids = Int32Array::from(vec![1, 2]);
        let names = StringArray::from(vec!["a", "b"]);
        let entries = StructArray::from(vec![
            (
                Arc::new(arrow_schema::Field::new(
                    "id",
                    arrow_schema::DataType::Int32,
                    false,
                )),
                Arc::new(ids) as ArrayRef,
            ),
            (
                Arc::new(arrow_schema::Field::new(
                    "name",
                    arrow_schema::DataType::Utf8,
                    false,
                )),
                Arc::new(names) as ArrayRef,
            ),
        ]);
        let offsets = arrow_buffer::OffsetBuffer::new(vec![0, 1, 2].into());
        let child = Arc::new(arrow_schema::Field::new(
            "item",
            entries.data_type().clone(),
            true,
        ));
        let list: ArrayRef = Arc::new(ListArray::new(child, offsets, Arc::new(entries), None));

        let target = DataType::from_str("serie<struct<ID: int64>>")
            .unwrap()
            .nullable_field("rows");
        let narrowed = cast(&target, list).unwrap();
        let arrow_schema::DataType::List(item) = narrowed.data_type() else {
            panic!("expected a list, got {:?}", narrowed.data_type());
        };
        let arrow_schema::DataType::Struct(fields) = item.data_type() else {
            panic!("expected a struct item, got {:?}", item.data_type());
        };
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].name(), "ID");
        assert_eq!(fields[0].data_type(), &arrow_schema::DataType::Int64);
    }

    #[test]
    fn a_map_wrapper_change_stays_refused_with_the_reconciliation_reason() {
        // A Map target from a non-Map source would bypass key/value semantics;
        // the guard names why instead of letting positional casting run.
        // The core reconciles map children on the conventional key/value names.
        let names = arrow_array::builder::MapFieldNames {
            entry: "entries".into(),
            key: "key".into(),
            value: "value".into(),
        };
        let mut builder = MapBuilder::new(Some(names), StringBuilder::new(), Int32Builder::new());
        builder.keys().append_value("k");
        builder.values().append_value(1);
        builder.append(true).unwrap();
        let map: ArrayRef = Arc::new(builder.finish());

        // Map -> Map with a cast value type goes through the dedicated arm.
        let widened = cast(
            &DataType::from_str("map<utf8, int64>")
                .unwrap()
                .nullable_field("m"),
            map,
        )
        .unwrap();
        assert!(matches!(
            widened.data_type(),
            arrow_schema::DataType::Map(..)
        ));

        // Utf8 -> Map is not a cast anything defines; the refusal says so.
        let refused = cast(
            &DataType::from_str("map<utf8, int64>")
                .unwrap()
                .nullable_field("m"),
            Arc::new(StringArray::from(vec!["x"])) as ArrayRef,
        );
        assert!(refused.is_err());
    }

    #[test]
    fn unsafe_and_safe_disagree_exactly_where_a_value_cannot_convert() {
        let source: ArrayRef = Arc::new(StringArray::from(vec![Some("12"), Some("nope")]));
        let target = DataType::Int64.nullable_field("n");

        // Unsafe: the unconvertible value is an error naming the cast.
        assert!(cast(&target, Arc::clone(&source)).is_err());

        // Safe: it becomes null instead.
        let softened = cast_with(&target, source, ArrowCastOptions::new()).unwrap();
        let softened = softened.as_any().downcast_ref::<Int64Array>().unwrap();
        assert_eq!(softened.value(0), 12);
        assert!(softened.is_null(1));
    }

    #[test]
    fn temporal_text_reads_this_crates_spellings_and_keeps_arrows() {
        // The crate's own spellings, which Arrow's kernel refuses: a grouped
        // fraction, an hour past the end of the day, a bracketed zone name, and a
        // duration in either spelling - Arrow reads no text into a duration.
        let clocks: ArrayRef = Arc::new(StringArray::from(vec![
            Some("10:00:00.000_001"),
            Some("25:30:00"),
            None,
        ]));
        let read = cast(
            &DataType::time64(TimeUnit::Microsecond)
                .unwrap()
                .nullable_field("clock"),
            clocks,
        )
        .unwrap();
        assert_eq!(
            read.as_any()
                .downcast_ref::<Time64MicrosecondArray>()
                .unwrap()
                .iter()
                .collect::<Vec<_>>(),
            [Some(36_000_000_001), Some(5_400_000_000), None]
        );

        let elapsed: ArrayRef = Arc::new(StringArray::from(vec!["PT90S", "26:03:04", "-01:30:00"]));
        let read = cast(
            &DataType::duration64(TimeUnit::Second)
                .unwrap()
                .nullable_field("took"),
            elapsed,
        )
        .unwrap();
        assert_eq!(
            read.as_any()
                .downcast_ref::<DurationSecondArray>()
                .unwrap()
                .values(),
            &[90, 93_784, -5_400]
        );

        // A named zone reads its bracket, and the count is the instant.
        let instants: ArrayRef = Arc::new(StringArray::from(vec![
            "2026-08-17T10:00:00+02:00[Europe/Paris]",
        ]));
        let paris = DataType::DateTime64 {
            unit: TimeUnit::Second,
            timezone: Timezone::from_str("Europe/Paris").unwrap(),
        };
        let read = cast(&paris.nullable_field("at"), instants).unwrap();
        assert_eq!(
            read.as_any()
                .downcast_ref::<TimestampSecondArray>()
                .unwrap()
                .values(),
            &[1_786_953_600]
        );

        // Arrow's own spellings keep reading: this widens the grammar, never
        // narrows it.
        let loose: ArrayRef = Arc::new(StringArray::from(vec!["10:23", "10:23:45 PM"]));
        let read = cast(
            &DataType::time32(TimeUnit::Second)
                .unwrap()
                .nullable_field("clock"),
            loose,
        )
        .unwrap();
        assert_eq!(
            read.as_any()
                .downcast_ref::<Time32SecondArray>()
                .unwrap()
                .values(),
            &[37_380, 80_625]
        );

        // A dictionary of text reads the same way its plain column does.
        let keys = Int32Array::from(vec![0, 1, 0]);
        let values: ArrayRef = Arc::new(StringArray::from(vec!["25:30:00", "00:00:01"]));
        let encoded: ArrayRef = Arc::new(DictionaryArray::new(keys, values));
        let read = cast(
            &DataType::time32(TimeUnit::Second)
                .unwrap()
                .nullable_field("clock"),
            encoded,
        )
        .unwrap();
        assert_eq!(
            read.as_any()
                .downcast_ref::<Time32SecondArray>()
                .unwrap()
                .iter()
                .collect::<Vec<_>>(),
            [Some(5_400), Some(1), Some(5_400)]
        );
    }

    #[test]
    fn temporal_text_neither_reading_takes_names_its_row() {
        let field = DataType::time32(TimeUnit::Second)
            .unwrap()
            .nullable_field("clock");
        let refused: ArrayRef = Arc::new(StringArray::from(vec!["10:23:45", "later"]));

        let message = cast(&field, Arc::clone(&refused)).unwrap_err().to_string();
        assert!(message.contains("row 1"), "{message}");
        assert!(message.contains("later"), "{message}");

        // The safe cast nulls the same row instead.
        let read = cast_with(&field, refused, ArrowCastOptions::new()).unwrap();
        assert_eq!(
            read.as_any()
                .downcast_ref::<Time32SecondArray>()
                .unwrap()
                .iter()
                .collect::<Vec<_>>(),
            [Some(37_425), None]
        );
    }

    #[test]
    fn a_reading_this_crate_refuses_is_never_arrows_rounded_one() {
        // Half a second is no whole second: this crate reads the spelling and
        // refuses the count, so the row and the column agree on null rather than
        // taking Arrow's truncation.
        let field = DataType::time32(TimeUnit::Second)
            .unwrap()
            .nullable_field("clock");
        let inexact: ArrayRef = Arc::new(StringArray::from(vec!["00:00:00.500", "10:23"]));
        let read = cast_with(&field, inexact, ArrowCastOptions::new()).unwrap();
        assert_eq!(
            read.as_any()
                .downcast_ref::<Time32SecondArray>()
                .unwrap()
                .iter()
                .collect::<Vec<_>>(),
            // The second value is a spelling only Arrow reads, so Arrow reads it.
            [None, Some(37_380)]
        );
    }

    #[test]
    fn an_encoded_temporal_column_reads_and_spells_like_a_plain_one() {
        // A dictionary is a layout, not a reading: the values read through this
        // crate's spellings and are encoded afterwards.
        let text: ArrayRef = Arc::new(StringArray::from(vec!["25:30:00", "10:00:00.000_001"]));
        let encoded = DataType::dictionary(
            DataType::Int32,
            DataType::time64(TimeUnit::Microsecond).unwrap(),
        )
        .unwrap();
        let read = cast(&encoded.nullable_field("clock"), text).unwrap();
        let read = read
            .as_any()
            .downcast_ref::<DictionaryArray<arrow_array::types::Int32Type>>()
            .unwrap();
        assert_eq!(
            read.values()
                .as_any()
                .downcast_ref::<Time64MicrosecondArray>()
                .unwrap()
                .values(),
            &[5_400_000_000, 36_000_000_001]
        );

        // The spelling direction unwraps the same layouts.
        let keys = Int32Array::from(vec![0, 0]);
        let values: ArrayRef =
            Arc::new(TimestampSecondArray::from(vec![1_700_000_000]).with_timezone("Europe/Paris"));
        let instants: ArrayRef = Arc::new(DictionaryArray::new(keys, values));
        let text = cast(&DataType::utf8().nullable_field("at"), instants).unwrap();
        assert_eq!(
            text.as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(1),
            "2023-11-14T23:13:20+01:00[Europe/Paris]"
        );
    }

    #[test]
    fn a_zone_arrow_cannot_name_never_sinks_this_crates_reading() {
        // Arrow parses a target zone once for the whole column and refuses a named
        // one, so its failure must leave the values this crate read standing.
        let paris = DataType::DateTime64 {
            unit: TimeUnit::Second,
            timezone: Timezone::from_str("Europe/Paris").unwrap(),
        };
        let mixed: ArrayRef = Arc::new(StringArray::from(vec![
            "2026-08-17T10:00:00+02:00",
            "not an instant",
        ]));
        let read = cast_with(&paris.nullable_field("at"), mixed, ArrowCastOptions::new()).unwrap();
        assert_eq!(
            read.as_any()
                .downcast_ref::<TimestampSecondArray>()
                .unwrap()
                .iter()
                .collect::<Vec<_>>(),
            [Some(1_786_953_600), None]
        );
    }

    #[test]
    fn temporals_render_the_spelling_this_crate_prints() {
        // A zoned instant renders its offset and its zone name, which Arrow's own
        // formatter cannot spell without a timezone database.
        let at: ArrayRef =
            Arc::new(TimestampSecondArray::from(vec![1_700_000_000]).with_timezone("Europe/Paris"));
        let text = cast(&DataType::utf8().nullable_field("at"), at).unwrap();
        assert_eq!(
            text.as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(0),
            "2023-11-14T23:13:20+01:00[Europe/Paris]"
        );

        // The other families spell what an expression literal spells.
        let elapsed: ArrayRef = Arc::new(DurationMillisecondArray::from(vec![90_000, -1_500]));
        let text = cast(&DataType::utf8().nullable_field("took"), elapsed).unwrap();
        let text = text.as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!((text.value(0), text.value(1)), ("PT90.000S", "-PT1.500S"));

        let clock: ArrayRef = Arc::new(Time64NanosecondArray::from(vec![Some(1), None]));
        let text = cast(&DataType::utf8().nullable_field("clock"), clock).unwrap();
        let text = text.as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(text.value(0), "00:00:00.000000001");
        assert!(text.is_null(1));
    }
}

mod plans {
    use std::sync::Arc;

    use super::root;
    use arrow_array::{
        ArrayRef, Int32Array, Int64Array, RecordBatch, RecordBatchReader, StringArray,
    };
    use arrow_schema::{
        ArrowError, DataType as ArrowDataType, Field as ArrowField, Schema, SchemaRef,
    };
    use yggdryl::arrow::BatchReader;
    use yggdryl::{
        ArrowCastOptions, ArrowCastPlan, DataType, Field, Nullability, Serie, SerieReader,
        StructType,
    };

    fn stored() -> SchemaRef {
        Arc::new(Schema::new(vec![
            ArrowField::new("id", ArrowDataType::Int32, false),
            ArrowField::new("symbol", ArrowDataType::Utf8, true),
        ]))
    }

    fn batch(offset: i32) -> RecordBatch {
        RecordBatch::try_new(
            stored(),
            vec![
                Arc::new(Int32Array::from(vec![offset, offset + 1])),
                Arc::new(StringArray::from(vec!["AAPL", "MSFT"])),
            ],
        )
        .unwrap()
    }

    fn target() -> Field {
        root([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
        ])
    }

    /// The record field a batch of `schema` lays out as: the plan's source.
    fn source_of(schema: &Schema) -> Field {
        Field::from_arrow_schema("row", schema).unwrap()
    }

    /// The record column of `batch`'s rows, sharing its columns.
    fn serie(batch: &RecordBatch) -> Serie {
        Serie::from_arrow_batch(None, batch, ArrowCastOptions::new()).unwrap()
    }

    /// A reader that counts what has been pulled and refuses to be pulled again
    /// once it has been dropped - which is how "released early" is observed.
    struct Counted {
        schema: SchemaRef,
        remaining: usize,
        pulled: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Iterator for Counted {
        type Item = std::result::Result<RecordBatch, ArrowError>;

        fn next(&mut self) -> Option<Self::Item> {
            if self.remaining == 0 {
                return None;
            }
            self.remaining -= 1;
            self.pulled
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Some(Ok(batch(0)))
        }
    }

    impl RecordBatchReader for Counted {
        fn schema(&self) -> SchemaRef {
            Arc::clone(&self.schema)
        }
    }

    impl Drop for Counted {
        fn drop(&mut self) {
            self.pulled
                .fetch_add(1_000, std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn counted(batches: usize) -> (BatchReader, Arc<std::sync::atomic::AtomicUsize>) {
        let pulled = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let reader = Counted {
            schema: stored(),
            remaining: batches,
            pulled: Arc::clone(&pulled),
        };
        (Box::new(reader), pulled)
    }

    #[test]
    fn one_plan_answers_every_batch_of_its_schema() {
        let plan =
            ArrowCastPlan::compile(&source_of(&stored()), &target(), ArrowCastOptions::new())
                .unwrap();
        let schema = plan.as_target().clone().into_arrow_schema().unwrap();
        assert_eq!(schema.field(0).data_type(), &ArrowDataType::Int64);
        assert_eq!(plan.as_target(), &target());
        assert_eq!(
            plan.as_source().data_type(),
            &ArrowDataType::Struct(stored().fields().clone())
        );
        assert_eq!(plan.as_options(), &ArrowCastOptions::new());
        assert!(!plan.is_identity());

        // Reusing the plan and casting each batch on its own are the same answer.
        for offset in 0..4 {
            let source = batch(offset);
            let planned = plan
                .apply(&serie(&source))
                .unwrap()
                .into_arrow_batch()
                .unwrap();
            let alone = Serie::from_arrow_batch(Some(&target()), &source, ArrowCastOptions::new())
                .unwrap()
                .into_arrow_batch()
                .unwrap();
            assert_eq!(planned, alone);
        }
    }

    #[test]
    fn a_plan_refuses_a_batch_of_another_schema() {
        let plan =
            ArrowCastPlan::compile(&source_of(&stored()), &target(), ArrowCastOptions::new())
                .unwrap();
        let other = RecordBatch::try_new(
            Arc::new(Schema::new(vec![ArrowField::new(
                "id",
                ArrowDataType::Int64,
                false,
            )])),
            vec![Arc::new(Int64Array::from(vec![1]))],
        )
        .unwrap();

        let message = plan.apply(&serie(&other)).unwrap_err().to_string();
        assert!(
            message.contains("this cast plan was compiled for"),
            "{message}"
        );
    }

    #[test]
    fn an_exact_plan_hands_the_caller_its_own_buffers_back() {
        let exact = target();
        let schema = exact.clone().into_arrow_schema().unwrap();
        let source = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Int64Array::from(vec![1, 2])),
                Arc::new(StringArray::from(vec![Some("AAPL"), None])),
            ],
        )
        .unwrap();

        let plan =
            ArrowCastPlan::compile(&source_of(&schema), &exact, ArrowCastOptions::new()).unwrap();
        assert!(plan.is_identity());
        let cast = plan
            .apply(&serie(&source))
            .unwrap()
            .into_arrow_batch()
            .unwrap();

        // Not merely equal: the same schema and the same buffers. A column
        // holds each child as its own typed array, so the `Arc` around it is
        // new; the allocations under it are the caller's.
        assert_eq!(cast.schema(), schema);
        for (before, after) in source.columns().iter().zip(cast.columns()) {
            assert!(before.to_data().ptr_eq(&after.to_data()));
        }
    }

    #[test]
    fn preflight_reports_a_schema_failure_and_leaves_the_row_failures_alone() {
        let missing = root([
            DataType::Int64.required_field("id"),
            DataType::utf8().required_field("venue"),
        ]);
        let strict = ArrowCastOptions::new().with_nullability(Nullability::Strict);

        // A required column no source carries is a schema failure, so it never
        // reaches preflight: compiling already refused it.
        assert!(ArrowCastPlan::compile(&source_of(&stored()), &missing, strict).is_err());

        // A null in a required column is a row failure, so an empty preflight
        // passes and the refusal waits for a batch that has rows.
        let required = root([
            DataType::Int64.required_field("id"),
            DataType::utf8().required_field("symbol"),
        ]);
        let plan = ArrowCastPlan::compile(&source_of(&stored()), &required, strict).unwrap();
        plan.preflight().unwrap();

        let with_null = RecordBatch::try_new(
            stored(),
            vec![
                Arc::new(Int32Array::from(vec![1])),
                Arc::new(StringArray::from(vec![None::<&str>])),
            ],
        )
        .unwrap();
        assert!(plan.apply(&serie(&with_null)).is_err());
    }

    #[test]
    fn a_reader_plans_once_and_casts_when_a_batch_is_pulled() {
        let (inner, pulled) = counted(3);
        let reader =
            SerieReader::from_arrow_reader(Some(&target()), inner, ArrowCastOptions::new())
                .unwrap()
                .into_arrow_reader();

        // The schema is answered before anything is pulled.
        assert_eq!(reader.schema().field(0).data_type(), &ArrowDataType::Int64);
        assert_eq!(pulled.load(std::sync::atomic::Ordering::Relaxed), 0);

        let batches: Vec<_> = reader.collect::<std::result::Result<_, _>>().unwrap();
        assert_eq!(batches.len(), 3);
        // Three pulls, then the drop marker: nothing was collected up front and
        // the source did not outlive its last batch.
        assert_eq!(pulled.load(std::sync::atomic::Ordering::Relaxed), 1_003);
    }

    #[test]
    fn an_exact_reader_is_the_reader_itself() {
        let exact = Field::new(
            "row",
            StructType::from_fields([
                DataType::Int32.required_field("id"),
                DataType::utf8().nullable_field("symbol"),
            ])
            .map(DataType::from)
            .unwrap(),
            false,
        );
        let (inner, pulled) = counted(1);
        let reader = SerieReader::from_arrow_reader(Some(&exact), inner, ArrowCastOptions::new())
            .unwrap()
            .into_arrow_reader();
        drop(reader);

        // Nothing wrapped it, so dropping it dropped the source directly.
        assert_eq!(pulled.load(std::sync::atomic::Ordering::Relaxed), 1_000);
    }

    #[test]
    fn a_strict_reader_reports_its_batch_at_the_pull_and_then_fuses() {
        let broken = RecordBatch::try_new(
            stored(),
            vec![
                Arc::new(Int32Array::from(vec![1, 2])),
                Arc::new(StringArray::from(vec![Some("AAPL"), None])),
            ],
        )
        .unwrap();
        let required = root([
            DataType::Int64.required_field("id"),
            DataType::utf8().required_field("symbol"),
        ]);
        let inner = yggdryl::arrow::batch_reader(stored(), [batch(0), broken, batch(9)]);
        let mut reader = SerieReader::from_arrow_reader(
            Some(&required),
            inner,
            ArrowCastOptions::new().with_nullability(Nullability::Strict),
        )
        .unwrap()
        .into_arrow_reader();

        assert!(reader.next().unwrap().is_ok());
        let error = reader.next().unwrap().unwrap_err();
        assert!(error.to_string().contains("$.symbol"), "{error}");
        // Fused: the third batch is never asked for, because the stream already
        // reported that it could not be honoured.
        assert!(reader.next().is_none());
    }

    #[test]
    fn dropping_a_reader_early_releases_the_source() {
        let (inner, pulled) = counted(100);
        let mut reader =
            SerieReader::from_arrow_reader(Some(&target()), inner, ArrowCastOptions::new())
                .unwrap()
                .into_arrow_reader();
        assert!(reader.next().unwrap().is_ok());
        drop(reader);

        // One pull, then the drop marker: the other 99 batches were never decoded.
        assert_eq!(pulled.load(std::sync::atomic::Ordering::Relaxed), 1_001);
    }

    #[test]
    fn a_compiled_plan_crosses_threads() {
        fn assert_send_sync<T: Send + Sync>(_: &T) {}

        let plan = Arc::new(
            ArrowCastPlan::compile(&source_of(&stored()), &target(), ArrowCastOptions::new())
                .unwrap(),
        );
        assert_send_sync(&plan);

        let handles: Vec<_> = (0..4)
            .map(|offset| {
                let plan = Arc::clone(&plan);
                std::thread::spawn(move || plan.apply(&serie(&batch(offset))).unwrap().len())
            })
            .collect();
        for handle in handles {
            assert_eq!(handle.join().unwrap(), 2);
        }
    }

    #[test]
    fn a_cast_column_is_the_only_thing_a_plan_rebuilds() {
        let plan =
            ArrowCastPlan::compile(&source_of(&stored()), &target(), ArrowCastOptions::new())
                .unwrap();
        let source = batch(0);
        let cast = plan
            .apply(&serie(&source))
            .unwrap()
            .into_arrow_batch()
            .unwrap();

        // `id` widened, so it is new buffers; `symbol` was already exact and is
        // the very buffers the caller handed over.
        assert!(!cast.column(0).to_data().ptr_eq(&source.column(0).to_data()));
        assert!(cast.column(1).to_data().ptr_eq(&source.column(1).to_data()));
        let ids: &Int64Array = cast.column(0).as_any().downcast_ref().unwrap();
        assert_eq!(ids.values(), &[0, 1]);
        let _: &ArrayRef = cast.column(1);
    }

    #[test]
    fn the_four_cast_doors_are_the_same_cast_at_four_widths() {
        // `Serie`'s Arrow doors carry the cast, and `cast_scalar` carries it for
        // one value, so every leaf answers it and the root answers it the same
        // way. What the four doors differ in is only what they are handed: a
        // value, an array, a batch, a stream.
        use yggdryl::arrow::batch_reader;

        let field = target();

        // A batch, and a reader over batches of the same schema. The stream is the
        // batch door repeated, so the two agree column for column.
        let cast = Serie::from_arrow_batch(Some(&field), &batch(0), ArrowCastOptions::new())
            .unwrap()
            .into_arrow_batch()
            .unwrap();
        assert_eq!(cast.column(0).data_type(), &ArrowDataType::Int64);

        let (reader, _) = counted(2);
        let streamed =
            SerieReader::from_arrow_reader(Some(&field), reader, ArrowCastOptions::new())
                .unwrap()
                .into_arrow_reader();
        assert_eq!(streamed.schema(), cast.schema());
        let pulled: Vec<_> = streamed.map(std::result::Result::unwrap).collect();
        assert_eq!(pulled.len(), 2);
        assert_eq!(pulled[0], cast);

        // A reader already carrying the declared shape is handed straight back,
        // because casting it would rebuild arrays it would hand back unchanged.
        let exact = batch_reader(cast.schema(), [cast.clone()]);
        let same: Vec<_> =
            SerieReader::from_arrow_reader(Some(&field), exact, ArrowCastOptions::new())
                .unwrap()
                .into_arrow_reader()
                .map(std::result::Result::unwrap)
                .collect();
        assert_eq!(same, vec![cast.clone()]);

        // An array and a one-row scalar, against the child that column is.
        let child = DataType::Int64.required_field("id");
        let column: ArrayRef = Arc::new(Int32Array::from(vec![7]));
        let ids =
            Serie::from_arrow_array(Some(&child), Arc::clone(&column), ArrowCastOptions::new())
                .unwrap()
                .require_arrow_array()
                .unwrap();
        assert_eq!(
            ids.as_ref(),
            &Int64Array::from(vec![7]) as &dyn arrow_array::Array
        );
        let scalar =
            Serie::from_arrow_array(Some(&child), Arc::clone(&column), ArrowCastOptions::new())
                .unwrap()
                .into_arrow_scalar()
                .unwrap();
        assert_eq!(arrow_array::Datum::get(&scalar).0, ids.as_ref());
        // A scalar is one row, and says so when it is handed more.
        assert!(
            Serie::from_arrow_array(
                Some(&child),
                Arc::new(Int32Array::from(vec![7, 8])) as ArrayRef,
                ArrowCastOptions::new()
            )
            .unwrap()
            .into_arrow_scalar()
            .is_err()
        );

        // A bare datatype is carried as its required `value` field, and a value
        // crosses the same boundary through `cast_scalar`.
        let widened = Serie::from_arrow_array(
            Some(&DataType::Int64.required_field("value")),
            Arc::clone(&column),
            ArrowCastOptions::new(),
        )
        .unwrap()
        .require_arrow_array()
        .unwrap();
        assert_eq!(widened.as_ref(), ids.as_ref());
        assert_eq!(
            DataType::Int64
                .cast_scalar(&yggdryl::Scalar::from(7_i32))
                .unwrap(),
            yggdryl::Scalar::from(7_i64)
        );

        // A field states one thing its datatype does not, and refuses on it.
        assert!(child.cast_scalar(&yggdryl::Scalar::Null).is_err());
        assert!(
            DataType::Int64
                .nullable_field("id")
                .cast_scalar(&yggdryl::Scalar::Null)
                .is_ok()
        );
    }
}

mod batches {
    use std::sync::Arc;

    use arrow_array::{Int32Array, RecordBatch, StringArray};
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
    use yggdryl::{ArrowCastOptions, DataType, Field, Serie, StructType};

    fn root(fields: impl IntoIterator<Item = Field>) -> Field {
        Field::new(
            "row",
            DataType::from(StructType::from_fields(fields).unwrap()),
            false,
        )
    }

    #[test]
    fn a_missing_column_is_filled_with_its_canonical_default() {
        let source = Arc::new(Schema::new(vec![ArrowField::new(
            "id",
            ArrowDataType::Int32,
            false,
        )]));
        let batch =
            RecordBatch::try_new(source, vec![Arc::new(Int32Array::from(vec![1, 2]))]).unwrap();

        let target = root([
            DataType::Int64.required_field("id"),
            DataType::utf8().required_field("symbol"),
        ]);
        let cast = Serie::from_arrow_batch(Some(&target), &batch, ArrowCastOptions::new())
            .unwrap()
            .into_arrow_batch()
            .unwrap();

        assert_eq!(cast.num_columns(), 2);
        assert_eq!(cast.num_rows(), 2);
        assert_eq!(cast.column(1).len(), 2);
        assert_eq!(cast.column(1).null_count(), 0);
    }

    #[test]
    fn columns_reconcile_by_name_and_extra_columns_are_dropped() {
        let source = Arc::new(Schema::new(vec![
            ArrowField::new("SYMBOL", ArrowDataType::Utf8, true),
            ArrowField::new("unused", ArrowDataType::Int32, true),
            ArrowField::new("ID", ArrowDataType::Int32, false),
        ]));
        let batch = RecordBatch::try_new(
            source,
            vec![
                Arc::new(StringArray::from(vec!["AAPL"])),
                Arc::new(Int32Array::from(vec![9])),
                Arc::new(Int32Array::from(vec![1])),
            ],
        )
        .unwrap();

        let target = root([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
        ]);
        let cast = Serie::from_arrow_batch(Some(&target), &batch, ArrowCastOptions::new())
            .unwrap()
            .into_arrow_batch()
            .unwrap();

        assert_eq!(cast.num_columns(), 2);
        assert_eq!(cast.schema().field(0).name(), "id");
        assert_eq!(cast.column(0).data_type(), &ArrowDataType::Int64);
    }

    #[test]
    fn an_exact_batch_keeps_its_own_arrays() {
        let target = root([DataType::Int32.required_field("id")]);
        let schema = Arc::new(Schema::new(vec![
            target.fields()[0].clone().into_arrow_field_ref().unwrap(),
        ]));
        let column: arrow_array::ArrayRef = Arc::new(Int32Array::from(vec![7]));
        let batch = RecordBatch::try_new(schema, vec![Arc::clone(&column)]).unwrap();

        let cast = Serie::from_arrow_batch(Some(&target), &batch, ArrowCastOptions::new())
            .unwrap()
            .into_arrow_batch()
            .unwrap();
        // A column holds its leaf as its own typed array, so the `Arc` around it
        // is new; the buffers under it are the caller's.
        assert!(cast.column(0).to_data().ptr_eq(&column.to_data()));
    }

    #[test]
    fn a_zero_column_batch_keeps_its_row_count() {
        let batch = RecordBatch::try_new_with_options(
            Arc::new(Schema::empty()),
            Vec::new(),
            &arrow_array::RecordBatchOptions::new().with_row_count(Some(3)),
        )
        .unwrap();

        let target = root([]);
        let cast = Serie::from_arrow_batch(Some(&target), &batch, ArrowCastOptions::new())
            .unwrap()
            .into_arrow_batch()
            .unwrap();
        assert_eq!(cast.num_rows(), 3);
        assert_eq!(cast.num_columns(), 0);
    }

    #[test]
    fn options_cast_is_declared_schema_then_selection_then_stored_completion() {
        use yggdryl::MimeType;
        use yggdryl::media::{IORecordOptions, RecordOptions};

        // Rows arrive as (symbol utf8, price int32, venue utf8).
        let source = Arc::new(Schema::new(vec![
            ArrowField::new("symbol", ArrowDataType::Utf8, false),
            ArrowField::new("price", ArrowDataType::Int32, false),
            ArrowField::new("venue", ArrowDataType::Utf8, false),
        ]));
        let batch = RecordBatch::try_new(
            source,
            vec![
                Arc::new(StringArray::from(vec!["AAPL"])),
                Arc::new(Int32Array::from(vec![12])),
                Arc::new(StringArray::from(vec!["XPAR"])),
            ],
        )
        .unwrap();

        // The declared schema widens the price; the selection narrows and
        // reorders; the stored shape finally adds the nullable column the
        // resource already has, as nulls, and every layer is one definition.
        let declared = root([
            DataType::utf8().required_field("symbol"),
            DataType::Int64.required_field("price"),
            DataType::utf8().required_field("venue"),
        ]);
        let stored = root([
            DataType::Int64.required_field("price"),
            DataType::utf8().required_field("symbol"),
            DataType::Int64.nullable_field("volume"),
        ]);
        let options = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)
            .unwrap()
            .with_field(declared)
            .with_select("PRICE, symbol")
            .unwrap();

        let shaped = options.apply_arrow_batch(batch, Some(&stored)).unwrap();

        let names: Vec<_> = shaped
            .schema()
            .fields()
            .iter()
            .map(|field| field.name().to_owned())
            .collect();
        assert_eq!(names, ["price", "symbol", "volume"]);
        assert_eq!(shaped.column(0).data_type(), &ArrowDataType::Int64);
        assert_eq!(shaped.num_rows(), 1);
        assert!(shaped.column(2).is_null(0));

        // A required stored column the rows do not carry is a declaration
        // they cannot meet, refused by name rather than written as its
        // canonical default.
        let required = root([
            DataType::Int64.required_field("price"),
            DataType::utf8().required_field("symbol"),
            DataType::Int64.required_field("volume"),
        ]);
        let batch = RecordBatch::try_new(
            Arc::new(Schema::new(vec![
                ArrowField::new("symbol", ArrowDataType::Utf8, false),
                ArrowField::new("price", ArrowDataType::Int32, false),
                ArrowField::new("venue", ArrowDataType::Utf8, false),
            ])),
            vec![
                Arc::new(StringArray::from(vec!["AAPL"])),
                Arc::new(Int32Array::from(vec![12])),
                Arc::new(StringArray::from(vec!["XPAR"])),
            ],
        )
        .unwrap();
        let refused = options
            .apply_arrow_batch(batch, Some(&required))
            .unwrap_err()
            .to_string();
        assert!(refused.contains("volume"), "{refused}");

        // A name the rows do not have is an error, not a null column.
        let missing = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)
            .unwrap()
            .with_select("absent")
            .unwrap();
        let empty = RecordBatch::new_empty(Arc::new(Schema::new(vec![ArrowField::new(
            "id",
            ArrowDataType::Int32,
            false,
        )])));
        let error = missing
            .apply_arrow_batch(empty, None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("absent"), "{error}");
    }
}

mod typed {
    use std::sync::Arc;

    use arrow_array::{
        Array, ArrayRef, BinaryArray, Datum, Float64Array, Int32Array, Int64Array, StringArray,
        UInt32Array, UInt64Array,
    };

    use yggdryl::cast::ArrowCastOptions;

    /// The reading that carries the bytes rather than the number they spell.
    fn bits() -> ArrowCastOptions {
        ArrowCastOptions::new().with_representation(yggdryl::Representation::Bits)
    }

    /// `array` cast into `field`, as the array of the column that comes out.
    fn cast_into(
        field: &Field,
        array: ArrayRef,
        options: ArrowCastOptions,
    ) -> yggdryl::arrow::Result<ArrayRef> {
        Ok(Serie::from_arrow_array(Some(field), array, options)?.require_arrow_array()?)
    }

    /// `array` cast into a bare datatype, which a cast carries as its required
    /// `value` field.
    fn cast_dtype(
        target: DataType,
        array: ArrayRef,
        options: ArrowCastOptions,
    ) -> yggdryl::arrow::Result<ArrayRef> {
        cast_into(&target.required_field("value"), array, options)
    }
    use yggdryl::FieldValue as _;
    use yggdryl::{DataType, EdgeAlgorithm, Field, Scalar, Serie, StructType};
    use yggdryl::{
        DateTimeField, GeometryField, Int32Field, Int64Field, StringField, StructField,
        UInt32Field, UInt64Field, VariantField,
    };
    use yggdryl::{TimeUnit, Timezone};

    #[test]
    fn a_typed_field_returns_its_own_array_type() {
        let field = Int64Field::new("id", yggdryl::Int64Type, false);
        let source: ArrayRef = Arc::new(Int32Array::from(vec![1, 2, 3]));

        // The column narrows to its leaf, whose buffers are an Int64Array; no
        // downcast at the call site.
        let cast = Serie::from_arrow_array(
            Some(&field.clone().into_field()),
            source,
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap();
        let ids: &Int64Array = cast.as_int64().expect("an int64 column").array();
        assert_eq!(ids.values(), &[1, 2, 3]);
    }

    #[test]
    fn a_string_field_parses_and_formats_through_the_same_call() {
        let field = StringField::try_new("symbol", DataType::utf8(), false).unwrap();
        let numbers: ArrayRef = Arc::new(Float64Array::from(vec![1.5, 2.5]));

        // A string's layout and charset decide its array, so the column is the
        // storage the field projects rather than one concrete array type.
        let cast = Serie::from_arrow_array(
            Some(&field.clone().into_field()),
            numbers,
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap()
        .require_arrow_array()
        .unwrap();
        let text = cast.as_any().downcast_ref::<StringArray>().unwrap();
        assert_eq!(text.value(0), "1.5");
        assert_eq!(text.value(1), "2.5");
    }

    #[test]
    fn an_unsafe_cast_fails_and_a_safe_one_defaults() {
        let field = Int64Field::new("id", yggdryl::Int64Type, false);
        let text: ArrayRef = Arc::new(StringArray::from(vec!["1", "not a number"]));

        assert!(
            Serie::from_arrow_array(
                Some(&field.clone().into_field()),
                Arc::clone(&text),
                ArrowCastOptions::new().with_safe(false)
            )
            .is_err()
        );

        // Safe casting nulls the failure, and a non-null field then defaults it.
        let cast = Serie::from_arrow_array(
            Some(&field.clone().into_field()),
            text,
            ArrowCastOptions::new(),
        )
        .unwrap();
        let ids = cast.as_int64().expect("an int64 column");
        assert_eq!(ids.values(), &[1, 0]);
        assert_eq!(ids.array().null_count(), 0);
    }

    #[test]
    fn a_nullable_field_keeps_the_null_a_safe_cast_produced() {
        let field = Int64Field::new("id", yggdryl::Int64Type, true);
        let text: ArrayRef = Arc::new(StringArray::from(vec!["1", "not a number"]));

        let ids = Serie::from_arrow_array(
            Some(&field.clone().into_field()),
            text,
            ArrowCastOptions::new(),
        )
        .unwrap();
        assert_eq!(ids.as_int64().expect("an int64 column").value(1), None);
        assert!(ids.is_null(1).unwrap());
    }

    #[test]
    fn a_struct_field_casts_children_by_name() {
        let field = StructField::try_from_field(Field::new(
            "row",
            StructType::from_fields([
                DataType::Int64.required_field("id"),
                DataType::utf8().nullable_field("symbol"),
            ])
            .map(DataType::from)
            .unwrap(),
            false,
        ))
        .unwrap();

        let source: ArrayRef = Arc::new(arrow_array::StructArray::from(vec![
            (
                Arc::new(arrow_schema::Field::new(
                    "id",
                    arrow_schema::DataType::Int32,
                    false,
                )),
                Arc::new(Int32Array::from(vec![7])) as ArrayRef,
            ),
            (
                Arc::new(arrow_schema::Field::new(
                    "symbol",
                    arrow_schema::DataType::Utf8,
                    true,
                )),
                Arc::new(StringArray::from(vec!["ACME"])) as ArrayRef,
            ),
        ]));

        let row = Serie::from_arrow_array(
            Some(&field.clone().into_field()),
            source,
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap();
        let row = row.as_struct().expect("a struct column");
        assert_eq!(row.children().len(), 2);
        assert!(row.child_at(0).unwrap().as_int64().is_some());
        assert_eq!(
            row.child_at(0)
                .unwrap()
                .require_arrow_array()
                .unwrap()
                .data_type(),
            &arrow_schema::DataType::Int64
        );
    }

    #[test]
    fn a_parameterized_temporal_field_casts_to_a_shared_array() {
        // A unit decides the physical width, so the column's array is an ArrayRef.
        let field = DateTimeField::try_new(
            "at",
            DataType::DateTime64 {
                unit: TimeUnit::Millisecond,
                timezone: Timezone::NAIVE,
            },
            false,
        )
        .unwrap();
        let source: ArrayRef = Arc::new(Int64Array::from(vec![1_700_000_000_000]));

        let cast: ArrayRef = Serie::from_arrow_array(
            Some(&field.clone().into_field()),
            source,
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap()
        .require_arrow_array()
        .unwrap();
        assert_eq!(cast.len(), 1);
        assert_eq!(
            cast.data_type(),
            &arrow_schema::DataType::Timestamp(arrow_schema::TimeUnit::Millisecond, None)
        );
    }

    #[test]
    fn a_scalar_cast_requires_exactly_one_value() {
        let field = Int64Field::new("id", yggdryl::Int64Type, false);
        let one: ArrayRef = Arc::new(Int32Array::from(vec![9]));
        let two: ArrayRef = Arc::new(Int32Array::from(vec![9, 10]));

        let scalar = Serie::from_arrow_array(
            Some(&field.clone().into_field()),
            one,
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap()
        .into_arrow_scalar()
        .unwrap();
        let (array, is_scalar) = scalar.get();
        assert!(is_scalar);
        assert_eq!(array.len(), 1);

        // The column's scalar door names the one row it takes and the count it
        // was handed.
        let message = Serie::from_arrow_array(
            Some(&field.clone().into_field()),
            two,
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap()
        .into_arrow_scalar()
        .unwrap_err()
        .to_string();
        assert!(message.contains("exactly one row, got 2"), "{message}");
    }

    #[test]
    fn a_borrowed_typed_field_casts_the_same_way() {
        let field = Int64Field::new("id", yggdryl::Int64Type, false);
        let borrowed = &field;
        let source: ArrayRef = Arc::new(Int32Array::from(vec![4]));

        assert_eq!(
            Serie::from_arrow_array(
                Some(&borrowed.clone().into_field()),
                source,
                ArrowCastOptions::new().with_safe(false)
            )
            .unwrap()
            .as_int64()
            .expect("an int64 column")
            .values(),
            &[4]
        );
    }

    #[test]
    fn bits_cover_the_full_32_bit_domain_in_both_directions_without_copying() {
        let source = UInt32Array::from(vec![0, 0x7fff_ffff, 0x8000_0000, u32::MAX]);
        let signed = Serie::from_arrow_array(
            Some(&Int32Field::new("digest", yggdryl::Int32Type, true).into_field()),
            Arc::new(source.clone()),
            bits(),
        )
        .unwrap();
        let signed = signed.as_int32().expect("an int32 column").array().clone();
        assert_eq!(signed.values(), &[0, i32::MAX, i32::MIN, -1]);
        assert!(
            signed.values().inner().ptr_eq(source.values().inner()),
            "reading the bits shares the physical value buffer"
        );

        let restored = Serie::from_arrow_array(
            Some(&UInt32Field::new("digest", yggdryl::UInt32Type, true).into_field()),
            Arc::new(signed.clone()),
            bits(),
        )
        .unwrap();
        let restored = restored.as_uint32().expect("a uint32 column").array();
        assert_eq!(restored.values(), source.values());
        assert!(
            restored.values().inner().ptr_eq(source.values().inner()),
            "the reverse reading retains the same physical buffer"
        );

        assert_eq!(
            Serie::from_arrow_array(
                Some(&Int32Field::new("digest", yggdryl::Int32Type, true).into_field()),
                Arc::new(UInt32Array::from(Vec::<u32>::new())),
                bits()
            )
            .unwrap()
            .as_int32()
            .expect("an int32 column")
            .values()
            .len(),
            0
        );
    }

    #[test]
    fn bits_cover_the_full_64_bit_domain_in_both_directions_without_copying() {
        let source = UInt64Array::from(vec![
            0,
            0x7fff_ffff_ffff_ffff,
            0x8000_0000_0000_0000,
            u64::MAX,
        ]);
        let signed = Serie::from_arrow_array(
            Some(&Int64Field::new("digest", yggdryl::Int64Type, true).into_field()),
            Arc::new(source.clone()),
            bits(),
        )
        .unwrap();
        let signed = signed.as_int64().expect("an int64 column").array().clone();
        assert_eq!(signed.values(), &[0, i64::MAX, i64::MIN, -1]);
        assert!(signed.values().inner().ptr_eq(source.values().inner()));

        let restored = Serie::from_arrow_array(
            Some(&UInt64Field::new("digest", yggdryl::UInt64Type, true).into_field()),
            Arc::new(signed),
            bits(),
        )
        .unwrap();
        let restored = restored.as_uint64().expect("a uint64 column").array();
        assert_eq!(restored.values(), source.values());
        assert!(restored.values().inner().ptr_eq(source.values().inner()));
    }

    #[test]
    fn eight_bytes_read_as_an_integer_a_float_or_bytes_alike() {
        use arrow_array::{FixedSizeBinaryArray, Float64Array};

        let source: ArrayRef = Arc::new(UInt64Array::from(vec![0, u64::MAX]));

        // The whole point of naming a width: an integer, its opposite sign, a
        // float and raw bytes are one buffer under four readings.
        let bytes = Serie::from_arrow_array(
            Some(&Field::new(
                "digest",
                DataType::fixed_binary(8).unwrap(),
                true,
            )),
            Arc::clone(&source),
            bits(),
        )
        .unwrap()
        .require_arrow_array()
        .unwrap();
        let stored: &FixedSizeBinaryArray = bytes.as_any().downcast_ref().unwrap();
        assert_eq!(stored.value(1), &[0xff; 8]);

        let floats = Serie::from_arrow_array(
            Some(&Field::new("digest", DataType::Float64, true)),
            Arc::clone(&bytes),
            bits(),
        )
        .unwrap()
        .require_arrow_array()
        .unwrap();
        let floats: &Float64Array = floats.as_any().downcast_ref().unwrap();
        assert!(floats.value(1).is_nan(), "{:?}", floats.value(1));

        // Round-tripping the whole chain restores the exact bit pattern.
        let restored = Serie::from_arrow_array(
            Some(&UInt64Field::new("digest", yggdryl::UInt64Type, true).into_field()),
            bytes,
            bits(),
        )
        .unwrap();
        let restored = restored.as_uint64().expect("a uint64 column").array();
        assert_eq!(restored.values(), &[0, u64::MAX]);
        assert!(
            restored
                .values()
                .inner()
                .ptr_eq(&source.to_data().buffers()[0]),
            "the bytes never left the buffer they arrived in"
        );
    }

    #[test]
    fn bits_preserve_slices_and_apply_the_target_null_contract() {
        let source = UInt64Array::from(vec![Some(3), Some(u64::MAX), None, Some(5)]).slice(1, 2);
        let nullable = Serie::from_arrow_array(
            Some(&Int64Field::new("digest", yggdryl::Int64Type, true).into_field()),
            Arc::new(source.clone()),
            bits(),
        )
        .unwrap();
        let nullable = nullable.as_int64().expect("an int64 column").array();
        assert_eq!(nullable.len(), 2);
        assert_eq!(nullable.value(0), -1);
        assert!(nullable.is_null(1));
        assert!(nullable.values().inner().ptr_eq(source.values().inner()));

        // The reading says what the bytes mean; the nullability policy still says
        // what an absent value means.
        let required = Serie::from_arrow_array(
            Some(&Int64Field::new("digest", yggdryl::Int64Type, false).into_field()),
            Arc::new(source.clone()),
            bits(),
        )
        .unwrap();
        let required = required.as_int64().expect("an int64 column").array();
        assert_eq!(required.values(), &[-1, 0]);
        assert_eq!(required.null_count(), 0);

        let refused = Serie::from_arrow_array(
            Some(&Int64Field::new("digest", yggdryl::Int64Type, false).into_field()),
            Arc::new(source),
            bits().with_nullability(yggdryl::Nullability::Strict),
        )
        .unwrap_err()
        .to_string();
        assert_eq!(refused, "required Arrow field $.digest holds 1 null values");
    }

    #[test]
    fn a_pair_that_is_not_the_same_bytes_converts_as_it_always_did() {
        // Asking for bits is a preference, not a mode: two widths that are not one
        // buffer take the ordinary numeric conversion, and its range check with it.
        let widened = Serie::from_arrow_array(
            Some(&Int64Field::new("id", yggdryl::Int64Type, true).into_field()),
            Arc::new(Int32Array::from(vec![7])),
            bits(),
        )
        .unwrap();
        assert_eq!(widened.as_int64().expect("an int64 column").values(), &[7]);

        let text = Serie::from_arrow_array(
            Some(&Field::new("id", DataType::utf8(), true)),
            Arc::new(Int64Array::from(vec![7])),
            bits(),
        )
        .unwrap()
        .require_arrow_array()
        .unwrap();
        assert_eq!(text.data_type(), &arrow_schema::DataType::Utf8);

        // A datatype whose values follow a rule keeps that rule: four bytes are
        // not US-ASCII text merely because they are four bytes.
        let refused = Serie::from_arrow_array(
            Some(&Field::new("ccy", DataType::fixed_ascii(4).unwrap(), true)),
            Arc::new(
                arrow_array::FixedSizeBinaryArray::try_from_iter([[0xff_u8; 4]].into_iter())
                    .unwrap(),
            ),
            bits().with_safe(false),
        )
        .unwrap_err()
        .to_string();
        assert!(refused.contains("ccy"), "{refused}");
    }

    #[test]
    fn ordinary_integer_casting_remains_numeric() {
        let field = Int64Field::new("digest", yggdryl::Int64Type, true);
        let source: ArrayRef = Arc::new(UInt64Array::from(vec![u64::MAX]));
        assert!(
            Serie::from_arrow_array(
                Some(&field.clone().into_field()),
                Arc::clone(&source),
                ArrowCastOptions::new().with_safe(false)
            )
            .is_err()
        );
        assert_eq!(
            Serie::from_arrow_array(Some(&field.clone().into_field()), source, bits())
                .unwrap()
                .as_int64()
                .expect("an int64 column")
                .value(0),
            Some(-1)
        );
    }

    /// One little-endian ISO WKB point.
    fn wkb_point(x: f64, y: f64) -> Vec<u8> {
        let mut bytes = vec![1u8, 1, 0, 0, 0];
        bytes.extend_from_slice(&x.to_le_bytes());
        bytes.extend_from_slice(&y.to_le_bytes());
        bytes
    }

    fn variant_storage_array(rows: usize) -> ArrayRef {
        let variants: Vec<yggdryl::Variant> = (0..rows)
            .map(|row| yggdryl::Variant::encode(&Scalar::from(row as i64)).unwrap())
            .collect();
        Arc::new(arrow_array::StructArray::new(
            arrow_schema::Fields::from(vec![
                arrow_schema::Field::new("metadata", arrow_schema::DataType::Binary, false),
                arrow_schema::Field::new("value", arrow_schema::DataType::Binary, false),
            ]),
            vec![
                Arc::new(BinaryArray::from_iter_values(
                    variants.iter().map(yggdryl::Variant::metadata),
                )) as ArrayRef,
                Arc::new(BinaryArray::from_iter_values(
                    variants.iter().map(yggdryl::Variant::value),
                )) as ArrayRef,
            ],
            None,
        ))
    }

    fn geospatial_batch(dtype: DataType, cells: Vec<Option<Vec<u8>>>) -> arrow_array::RecordBatch {
        let root = Field::new(
            "row",
            DataType::from(StructType::from_fields([Field::new("shape", dtype, true)]).unwrap()),
            false,
        );
        let schema = root.clone().into_arrow_schema().unwrap();
        let values: Vec<Option<&[u8]>> = cells.iter().map(|cell| cell.as_deref()).collect();
        arrow_array::RecordBatch::try_new(schema, vec![Arc::new(BinaryArray::from(values))])
            .unwrap()
    }

    fn cast_shape_to(
        batch: arrow_array::RecordBatch,
        target: Field,
    ) -> yggdryl::arrow::Result<arrow_array::RecordBatch> {
        let root = Field::new(
            "row",
            DataType::from(StructType::from_fields([target]).unwrap()),
            false,
        );
        Serie::from_arrow_batch(
            Some(&root),
            &batch,
            ArrowCastOptions::new().with_safe(false),
        )?
        .into_arrow_batch()
    }

    #[test]
    fn binary_bytes_entering_a_geometry_field_are_validated_as_wkb() {
        let field =
            GeometryField::try_new("shape", DataType::geometry(None).unwrap(), true).unwrap();
        let point = wkb_point(1.0, 2.0);
        let source: ArrayRef = Arc::new(BinaryArray::from(vec![Some(point.as_slice()), None]));

        // Valid WKB passes with the same bytes; the cast is the identity.
        let cast = Serie::from_arrow_array(
            Some(&field.to_field()),
            Arc::clone(&source),
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap();
        assert_eq!(
            cast.as_binary().expect("a WKB column").value(0),
            Some(point.as_slice())
        );
        let identity = cast.require_arrow_array().unwrap();
        assert!(identity.to_data().ptr_eq(&source.to_data()));

        // Truncated bytes are refused naming the field and the row.
        let broken: ArrayRef = Arc::new(BinaryArray::from(vec![Some([1u8, 1, 0].as_slice())]));
        let refused = Serie::from_arrow_array(
            Some(&field.to_field()),
            broken,
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap_err()
        .to_string();
        assert!(refused.contains("shape"), "{refused}");
        assert!(refused.contains("row 0"), "{refused}");
        assert!(refused.contains("WKB"), "{refused}");
    }

    #[test]
    fn a_geometry_column_renders_wkt_into_a_utf8_target() {
        let batch = geospatial_batch(
            DataType::geometry(None).unwrap(),
            vec![Some(wkb_point(1.0, 2.0)), None],
        );
        let cast = cast_shape_to(batch, Field::new("shape", DataType::utf8(), true)).unwrap();
        let text = cast
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .unwrap();
        assert_eq!(text.value(0), "POINT (1 2)");
        assert!(text.is_null(1));
    }

    #[test]
    fn a_geometry_column_stays_lossless_into_a_binary_target() {
        let point = wkb_point(3.0, 4.0);
        let batch = geospatial_batch(DataType::geometry(None).unwrap(), vec![Some(point.clone())]);
        let cast = cast_shape_to(batch, Field::new("shape", DataType::binary(), true)).unwrap();
        let bytes = cast
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        assert_eq!(bytes.value(0), point.as_slice());
    }

    #[test]
    fn a_crs_change_between_geospatial_columns_is_refused_naming_both() {
        let batch = geospatial_batch(
            DataType::geometry(None).unwrap(),
            vec![Some(wkb_point(1.0, 2.0))],
        );
        let refused = cast_shape_to(
            batch,
            Field::new(
                "shape",
                DataType::geometry(Some("EPSG:3857")).unwrap(),
                true,
            ),
        )
        .unwrap_err()
        .to_string();
        assert!(refused.contains("OGC:CRS84"), "{refused}");
        assert!(refused.contains("EPSG:3857"), "{refused}");
    }

    #[test]
    fn geometry_and_geography_refuse_each_other_naming_the_edge_change() {
        let batch = geospatial_batch(
            DataType::geometry(None).unwrap(),
            vec![Some(wkb_point(1.0, 2.0))],
        );
        let refused = cast_shape_to(
            batch,
            Field::new("shape", DataType::geography(None, None).unwrap(), true),
        )
        .unwrap_err()
        .to_string();
        assert!(refused.contains("edge"), "{refused}");

        let batch = geospatial_batch(
            DataType::geography(None, Some(EdgeAlgorithm::Spherical)).unwrap(),
            vec![Some(wkb_point(1.0, 2.0))],
        );
        let refused = cast_shape_to(
            batch,
            Field::new("shape", DataType::geometry(None).unwrap(), true),
        )
        .unwrap_err()
        .to_string();
        assert!(refused.contains("edge"), "{refused}");
    }

    #[test]
    fn a_matching_geospatial_pair_casts_as_the_identity() {
        let point = wkb_point(5.0, 6.0);
        let batch = geospatial_batch(DataType::geometry(None).unwrap(), vec![Some(point.clone())]);
        let cast = cast_shape_to(
            batch,
            Field::new("shape", DataType::geometry(None).unwrap(), true),
        )
        .unwrap();
        let bytes = cast
            .column(0)
            .as_any()
            .downcast_ref::<BinaryArray>()
            .unwrap();
        assert_eq!(bytes.value(0), point.as_slice());
    }

    #[test]
    fn text_into_a_geospatial_target_names_the_absent_wkt_parser() {
        let field = Field::new("shape", DataType::geometry(None).unwrap(), true);
        let source: ArrayRef = Arc::new(StringArray::from(vec!["POINT (1 2)"]));
        let refused = Serie::from_arrow_array(
            Some(&field),
            source,
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap_err()
        .to_string();
        assert!(refused.contains("WKT parser"), "{refused}");
    }

    #[test]
    fn a_variant_casts_only_from_its_own_storage() {
        let field = VariantField::new("payload", yggdryl::VariantType, true);
        let storage = variant_storage_array(2);

        // The identity works, and the column shares the caller's buffers.
        let cast = Serie::from_arrow_array(
            Some(&field.to_field()),
            Arc::clone(&storage),
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap();
        assert_eq!(cast.len(), 2);
        assert!(cast.as_variant().is_some());
        let identity = cast.require_arrow_array().unwrap();
        assert!(identity.to_data().ptr_eq(&storage.to_data()));

        // Anything else refuses by name: the column holds the two binaries
        // the encoding is, which a caller writes with `Variant::encode`.
        let numbers: ArrayRef = Arc::new(Int64Array::from(vec![7]));
        let refused = Serie::from_arrow_array(
            Some(&field.to_field()),
            numbers,
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap_err()
        .to_string();
        assert!(refused.contains("Variant::encode"), "{refused}");
    }

    #[test]
    fn a_variant_column_refuses_to_leave_the_type_by_a_cast() {
        let root = Field::new(
            "row",
            DataType::from(
                StructType::from_fields([Field::new("payload", DataType::variant(), true)])
                    .unwrap(),
            ),
            false,
        );
        let schema = root.clone().into_arrow_schema().unwrap();
        let batch =
            arrow_array::RecordBatch::try_new(schema, vec![variant_storage_array(1)]).unwrap();
        let target = Field::new(
            "row",
            DataType::from(
                StructType::from_fields([Field::new("payload", DataType::utf8(), true)]).unwrap(),
            ),
            false,
        );
        let refused = Serie::from_arrow_batch(
            Some(&target),
            &batch,
            ArrowCastOptions::new().with_safe(false),
        )
        .unwrap_err()
        .to_string();
        assert!(refused.contains("Variant::scalar"), "{refused}");
    }

    /// Every wrapper reads what the value inside it reads: a serie layout is a
    /// layout, an encoding is a layout, and a byte framing is a framing.
    mod layouts {
        use std::sync::Arc;

        use arrow_array::{
            ArrayRef, BinaryArray, FixedSizeBinaryArray, FixedSizeListArray, Int32Array,
            LargeListArray, ListArray, StringArray, StructArray,
        };
        use arrow_buffer::OffsetBuffer;
        use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Fields as ArrowFields};

        use super::cast_dtype;
        use yggdryl::DataType;
        use yggdryl::cast::ArrowCastOptions;

        fn dtype(expression: &str) -> DataType {
            expression.parse().unwrap()
        }

        fn strict() -> ArrowCastOptions {
            ArrowCastOptions::new().with_safe(false)
        }

        /// Two rows of two `{a: int32}` values, under every serie layout in turn.
        fn struct_items() -> (Arc<ArrowField>, ArrayRef) {
            let fields: ArrowFields =
                vec![Arc::new(ArrowField::new("a", ArrowDataType::Int32, true))].into();
            let values: ArrayRef = Arc::new(StructArray::new(
                fields.clone(),
                vec![Arc::new(Int32Array::from(vec![1, 2, 3, 4])) as ArrayRef],
                None,
            ));
            let item = Arc::new(ArrowField::new("item", ArrowDataType::Struct(fields), true));
            (item, values)
        }

        #[test]
        fn every_serie_layout_reads_every_other_one_through_a_struct_child() {
            let (item, values) = struct_items();
            let offsets = OffsetBuffer::new(vec![0, 2, 4].into());
            let sources: Vec<ArrayRef> = vec![
                Arc::new(
                    ListArray::try_new(
                        Arc::clone(&item),
                        offsets.clone(),
                        Arc::clone(&values),
                        None,
                    )
                    .unwrap(),
                ),
                Arc::new(
                    LargeListArray::try_new(
                        Arc::clone(&item),
                        OffsetBuffer::new(vec![0_i64, 2, 4].into()),
                        Arc::clone(&values),
                        None,
                    )
                    .unwrap(),
                ),
                Arc::new(
                    FixedSizeListArray::try_new(Arc::clone(&item), 2, Arc::clone(&values), None)
                        .unwrap(),
                ),
            ];
            let targets = [
                "serie<struct<a: int32>>",
                "large_serie<struct<a: int32>>",
                "serie_view<struct<a: int32>>",
                "large_serie_view<struct<a: int32>>",
                "fixed_size_serie<struct<a: int32>, 2>",
            ];
            for source in sources {
                for target in targets {
                    let cast = cast_dtype(dtype(target), Arc::clone(&source), strict())
                        .unwrap_or_else(|error| {
                            panic!("{:?} -> {target}: {error}", source.data_type())
                        });
                    assert_eq!(cast.len(), 2, "{target}");
                }
            }
        }

        #[test]
        fn two_fixed_sizes_are_a_row_change_and_say_so() {
            let (item, values) = struct_items();
            let source: ArrayRef =
                Arc::new(FixedSizeListArray::try_new(item, 2, values, None).unwrap());

            let refused = cast_dtype(
                dtype("fixed_size_serie<struct<a: int32>, 4>"),
                source,
                strict(),
            )
            .unwrap_err()
            .to_string();
            assert!(refused.contains("value change"), "{refused}");
        }

        #[test]
        fn an_encoded_target_runs_the_value_rule_its_leaf_carries() {
            let text: ArrayRef = Arc::new(StringArray::from(vec!["\u{e9}"]));
            for target in ["dictionary<int32, ascii>", "run_end_encoded<int32, ascii>"] {
                let refused = cast_dtype(dtype(target), Arc::clone(&text), strict())
                    .unwrap_err()
                    .to_string();
                assert!(refused.contains("non-ASCII byte"), "{target}: {refused}");
            }

            let wkb: ArrayRef = Arc::new(BinaryArray::from_vec(vec![b"nope"]));
            let refused = cast_dtype(dtype("dictionary<int32, geometry>"), wkb, strict())
                .unwrap_err()
                .to_string();
            assert!(refused.contains("WKB"), "{refused}");
        }

        #[test]
        fn an_encoded_target_reads_what_its_bare_leaf_reads() {
            let codes: ArrayRef = Arc::new(StringArray::from(vec!["US", "US", "FR"]));
            for target in [
                "dictionary<int32, country>",
                "run_end_encoded<int32, country>",
                "dictionary<int32, uuid>",
            ] {
                let source: ArrayRef = if target.contains("uuid") {
                    Arc::new(StringArray::from(vec![
                        "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
                    ]))
                } else {
                    Arc::clone(&codes)
                };
                let cast = cast_dtype(dtype(target), source, strict())
                    .unwrap_or_else(|error| panic!("{target}: {error}"));
                // The declared child field keeps the extension identity Arrow's
                // own encoding does not copy.
                assert_eq!(
                    &cast.data_type().clone(),
                    dtype(target)
                        .required_field("value")
                        .into_arrow_field_ref()
                        .unwrap()
                        .data_type(),
                    "{target}"
                );
            }
        }

        #[test]
        fn an_encoded_struct_source_is_decoded_and_reconciled_by_name() {
            use arrow_array::{DictionaryArray, Int32Array, RunArray, types::Int32Type};

            let fields: ArrowFields =
                vec![Arc::new(ArrowField::new("key", ArrowDataType::Utf8, true))].into();
            let values: ArrayRef = Arc::new(StructArray::new(
                fields,
                vec![Arc::new(StringArray::from(vec!["a", "b"])) as ArrayRef],
                None,
            ));
            let dictionary: ArrayRef = Arc::new(
                DictionaryArray::<Int32Type>::try_new(
                    Int32Array::from(vec![0, 1, 0]),
                    Arc::clone(&values),
                )
                .unwrap(),
            );
            let run: ArrayRef = Arc::new(
                RunArray::<Int32Type>::try_new(&Int32Array::from(vec![1, 2]), values.as_ref())
                    .unwrap(),
            );

            for source in [dictionary, run] {
                // The decode happens first, so the Struct child the encoding was
                // hiding is reconciled by name rather than positionally.
                let cast = cast_dtype(dtype("struct<KEY: utf8>"), Arc::clone(&source), strict())
                    .unwrap_or_else(|error| panic!("{:?}: {error}", source.data_type()));
                let ArrowDataType::Struct(cast_fields) = cast.data_type() else {
                    panic!("a struct target answers a struct");
                };
                assert_eq!(cast_fields[0].name(), "KEY");
            }
        }

        #[test]
        fn a_byte_framing_reaches_every_other_one_through_binary() {
            let text: ArrayRef = Arc::new(StringArray::from(vec!["abc"]));
            let fixed = cast_dtype(dtype("fixed_binary(3)"), Arc::clone(&text), strict()).unwrap();
            assert_eq!(
                fixed
                    .as_any()
                    .downcast_ref::<FixedSizeBinaryArray>()
                    .ok_or("downcast")
                    .unwrap()
                    .value(0),
                b"abc"
            );

            let back = cast_dtype(DataType::utf8(), fixed, strict()).unwrap();
            assert_eq!(
                back.as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap()
                    .value(0),
                "abc"
            );
        }
    }

    /// A string reads values under what the source declares and writes them
    /// under what the target declares: the layout, the charset and the bound.
    mod strings {

        /// Narrow an Arrow array the way any caller does, so the fixture does
        /// not borrow the crate's own internal narrowing.
        fn downcast<T: Array + 'static>(array: &dyn Array) -> Option<&T> {
            array.as_any().downcast_ref::<T>()
        }
        use std::sync::Arc;
        use yggdryl::StructType;

        use arrow_array::{Array, ArrayRef, BinaryArray, FixedSizeBinaryArray, StringArray};

        use super::{cast_dtype, cast_into};
        use yggdryl::cast::ArrowCastOptions;
        use yggdryl::{DataType, Field, Serie};

        fn dtype(expression: &str) -> DataType {
            expression.parse().unwrap()
        }

        fn strict() -> ArrowCastOptions {
            ArrowCastOptions::new().with_safe(false)
        }

        /// One column under a declared field, so its extension identity rides
        /// into the cast.
        fn batch(field: Field, column: ArrayRef) -> arrow_array::RecordBatch {
            let root = Field::new(
                "row",
                DataType::from(StructType::from_fields([field]).unwrap()),
                false,
            );
            let schema = root.clone().into_arrow_schema().unwrap();
            arrow_array::RecordBatch::try_new(schema, vec![column]).unwrap()
        }

        fn cast_column(
            source: arrow_array::RecordBatch,
            target: DataType,
            options: ArrowCastOptions,
        ) -> yggdryl::arrow::Result<ArrayRef> {
            let root = Field::new(
                "row",
                DataType::from(
                    StructType::from_fields([Field::new("text", target, true)]).unwrap(),
                ),
                false,
            );
            Ok(Arc::clone(
                Serie::from_arrow_batch(Some(&root), &source, options)?
                    .into_arrow_batch()?
                    .column(0),
            ))
        }

        #[test]
        fn a_bound_is_checked_on_the_way_in_and_a_failing_cell_is_null_when_safe() {
            let text: ArrayRef =
                Arc::new(StringArray::from(vec![Some("abc"), Some("abcdef"), None]));
            let refused = cast_dtype(dtype("utf8(4)"), Arc::clone(&text), strict())
                .unwrap_err()
                .to_string();
            assert!(refused.contains("row 1"), "{refused}");
            assert!(refused.contains("at most 4 bytes"), "{refused}");

            // A nullable field keeps the null a failing cell became.
            let lenient = cast_into(
                &Field::new("text", dtype("utf8(4)"), true),
                text,
                ArrowCastOptions::new(),
            )
            .unwrap();
            let lenient = lenient
                .as_ref()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            assert_eq!(lenient.value(0), "abc");
            assert!(lenient.is_null(1));
            assert!(lenient.is_null(2));
        }

        #[test]
        fn a_code_answers_safe_and_strict_exactly_as_a_string_does() {
            let text: ArrayRef = Arc::new(StringArray::from(vec![Some("USD"), Some("EURO"), None]));
            let refused = cast_dtype(DataType::Ccy, Arc::clone(&text), strict())
                .unwrap_err()
                .to_string();
            assert!(refused.contains("row 1"), "{refused}");
            assert!(refused.contains("at most 3 bytes"), "{refused}");

            let lenient = cast_into(
                &Field::new("ccy", DataType::Ccy, true),
                text,
                ArrowCastOptions::new(),
            )
            .unwrap();
            let lenient = lenient
                .as_ref()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            assert_eq!(lenient.value(0), "USD");
            assert!(lenient.is_null(1));
            assert!(lenient.is_null(2));

            // A text source every cell of which passes is the code's own
            // storage, so it is shared rather than copied. A column holds its
            // leaf as its own typed array, so the `Arc` around it is new; the
            // buffers under it are the caller's.
            let passing: ArrayRef = Arc::new(StringArray::from(vec!["USD", "EUR"]));
            let shared = cast_dtype(DataType::Ccy, Arc::clone(&passing), strict()).unwrap();
            assert!(shared.to_data().ptr_eq(&passing.to_data()));

            // A fixed binary source is trimmed of the padding its slot wrote and
            // stored as the text it spells.
            let stored: ArrayRef = Arc::new(
                FixedSizeBinaryArray::try_from_sparse_iter_with_size(
                    [Some(b"USD".as_slice()), Some(b"EU\xff".as_slice())].into_iter(),
                    3,
                )
                .unwrap(),
            );
            assert!(cast_dtype(DataType::Ccy, Arc::clone(&stored), strict()).is_err());
            let lenient = cast_into(
                &Field::new("ccy", DataType::Ccy, true),
                stored,
                ArrowCastOptions::new(),
            )
            .unwrap();
            let lenient = lenient
                .as_ref()
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap();
            assert_eq!(lenient.value(0), "USD");
            assert!(lenient.is_null(1));
        }

        #[test]
        fn a_charset_is_written_from_text_and_read_back_from_its_own_bytes() {
            let text: ArrayRef = Arc::new(StringArray::from(vec!["caf\u{e9}"]));
            let latin = batch(
                Field::new("text", dtype("string(windows-1252)"), true),
                cast_dtype(dtype("string(windows-1252)"), text, strict()).unwrap(),
            );
            let bytes = downcast::<BinaryArray>(latin.column(0).as_ref()).unwrap();
            assert_eq!(bytes.value(0), b"caf\xe9");

            // The recognized source is read under its own charset, so the
            // characters come back rather than the bytes.
            let back = cast_column(latin, DataType::utf8(), strict()).unwrap();
            assert_eq!(
                back.as_ref()
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap()
                    .value(0),
                "caf\u{e9}"
            );
        }

        #[test]
        fn a_scalar_the_target_charset_cannot_spell_fails_at_the_write() {
            // U+0101 has no windows-1252 byte, and the write seam is where that
            // is refused, naming the row.
            let text: ArrayRef = Arc::new(StringArray::from(vec!["\u{0101}"]));
            let refused = cast_dtype(dtype("cp1252"), text, strict())
                .unwrap_err()
                .to_string();
            assert!(refused.contains("row 0"), "{refused}");
        }

        #[test]
        fn a_fixed_width_pads_on_the_way_in_and_trims_on_the_way_out() {
            let text: ArrayRef = Arc::new(StringArray::from(vec!["ab"]));
            let fixed = cast_dtype(dtype("fixed_ascii(4)"), text, strict()).unwrap();
            assert_eq!(
                downcast::<FixedSizeBinaryArray>(fixed.as_ref())
                    .unwrap()
                    .value(0),
                b"ab\0\0"
            );

            let source = batch(Field::new("text", dtype("fixed_ascii(4)"), true), fixed);
            let back = cast_column(source, dtype("ascii"), strict()).unwrap();
            assert_eq!(
                back.as_ref()
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap()
                    .value(0),
                "ab"
            );
        }

        #[test]
        fn a_code_reads_into_a_string_and_bare_bytes_are_taken_as_the_target_charset() {
            let codes: ArrayRef = Arc::new(StringArray::from(vec!["USD"]));
            let currency = cast_dtype(DataType::Ccy, codes, strict()).unwrap();
            let source = batch(Field::new("text", DataType::Ccy, true), currency);
            let back = cast_column(source, dtype("utf8(8)"), strict()).unwrap();
            assert_eq!(
                back.as_ref()
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap()
                    .value(0),
                "USD"
            );

            let bytes: ArrayRef = Arc::new(BinaryArray::from_vec(vec![b"caf\xe9"]));
            let latin = cast_dtype(dtype("string(windows-1252)"), bytes, strict()).unwrap();
            assert_eq!(
                latin
                    .as_ref()
                    .as_any()
                    .downcast_ref::<BinaryArray>()
                    .unwrap()
                    .value(0),
                b"caf\xe9"
            );
        }
    }

    /// A byte column reads its cells only where it declares a maximum, which is
    /// the one thing about bytes Arrow has nowhere to state.
    mod bytes {

        /// Narrow an Arrow array the way any caller does, so the fixture does
        /// not borrow the crate's own internal narrowing.
        fn downcast<T: Array + 'static>(array: &dyn Array) -> Option<&T> {
            array.as_any().downcast_ref::<T>()
        }
        use std::sync::Arc;
        use yggdryl::StructType;

        use arrow_array::{Array, ArrayRef, BinaryArray, LargeBinaryArray, StringArray};

        use super::{cast_dtype, cast_into};
        use yggdryl::cast::ArrowCastOptions;
        use yggdryl::{DataType, Field, Serie};

        fn dtype(expression: &str) -> DataType {
            expression.parse().unwrap()
        }

        fn strict() -> ArrowCastOptions {
            ArrowCastOptions::new().with_safe(false)
        }

        /// One column under a declared field, so its extension identity rides
        /// into the cast.
        fn batch(field: Field, column: ArrayRef) -> arrow_array::RecordBatch {
            let root = Field::new(
                "row",
                DataType::from(StructType::from_fields([field]).unwrap()),
                false,
            );
            let schema = root.clone().into_arrow_schema().unwrap();
            arrow_array::RecordBatch::try_new(schema, vec![column]).unwrap()
        }

        #[test]
        fn a_maximum_is_checked_on_the_way_in_and_a_failing_cell_is_null_when_safe() {
            let cells: ArrayRef = Arc::new(BinaryArray::from(vec![
                Some(b"abc".as_slice()),
                Some(b"abcdef".as_slice()),
                None,
            ]));
            let refused = cast_dtype(dtype("binary(4)"), Arc::clone(&cells), strict())
                .unwrap_err()
                .to_string();
            assert!(refused.contains("row 1"), "{refused}");
            assert!(refused.contains("at most 4 bytes"), "{refused}");

            // A nullable field keeps the null a failing cell became.
            let lenient = cast_into(
                &Field::new("payload", dtype("binary(4)"), true),
                cells,
                ArrowCastOptions::new(),
            )
            .unwrap();
            let lenient = lenient
                .as_ref()
                .as_any()
                .downcast_ref::<BinaryArray>()
                .unwrap();
            assert_eq!(lenient.value(0), b"abc");
            assert!(lenient.is_null(1));
            assert!(lenient.is_null(2));
        }

        #[test]
        fn a_bounded_target_writes_its_own_layout_from_any_byte_source() {
            let text: ArrayRef = Arc::new(StringArray::from(vec!["ab"]));
            let large = cast_dtype(dtype("large_binary"), text, strict()).unwrap();
            assert_eq!(
                downcast::<LargeBinaryArray>(large.as_ref())
                    .unwrap()
                    .value(0),
                b"ab"
            );

            let fixed = cast_dtype(
                dtype("fixed_binary(3)"),
                Arc::new(BinaryArray::from_vec(vec![b"abc"])),
                strict(),
            )
            .unwrap();
            let view = cast_dtype(dtype("sized_binary(3)"), fixed, strict()).unwrap();
            assert_eq!(view.data_type(), &arrow_schema::DataType::Binary);
        }

        #[test]
        fn an_unbounded_layout_is_its_storage_and_a_declared_source_is_exact() {
            let cells: ArrayRef = Arc::new(BinaryArray::from_vec(vec![b"abc"]));
            // A column holds its leaf as its own typed array, so the `Arc`
            // around it is new; the buffers under it are the caller's.
            let plain = cast_dtype(DataType::binary(), Arc::clone(&cells), strict()).unwrap();
            assert!(plain.to_data().ptr_eq(&cells.to_data()));

            // A column written as `binary(4)` was measured when it was written,
            // so it comes back as the same array rather than a re-read one.
            let source = batch(Field::new("payload", dtype("binary(4)"), true), cells);
            let root = Field::new(
                "row",
                DataType::from(
                    StructType::from_fields([Field::new("payload", dtype("binary(4)"), true)])
                        .unwrap(),
                ),
                false,
            );
            let exact = Serie::from_arrow_batch(Some(&root), &source, strict())
                .unwrap()
                .into_arrow_batch()
                .unwrap();
            assert!(
                exact
                    .column(0)
                    .to_data()
                    .ptr_eq(&source.column(0).to_data())
            );
        }

        #[test]
        fn two_fixed_widths_are_a_value_change_and_say_so() {
            let fixed = cast_dtype(
                dtype("fixed_binary(3)"),
                Arc::new(BinaryArray::from_vec(vec![b"abc"])),
                strict(),
            )
            .unwrap();
            let refused = cast_dtype(dtype("fixed_binary(4)"), fixed, strict())
                .unwrap_err()
                .to_string();
            assert!(refused.contains("value change"), "{refused}");
        }
    }

    /// An empty text cell entering a column that holds neither text nor bytes is
    /// no value: null through every door, before any spelling is parsed and before
    /// `safe` is asked, so `nullability` alone decides what a required column does
    /// with it. A column that holds text or bytes keeps the cell as the value it is.
    mod empty_text {
        use std::sync::Arc;

        use arrow_array::types::{Int8Type, Int16Type};
        use arrow_array::{
            Array, ArrayRef, BinaryArray, DictionaryArray, Int16Array, Int32Array,
            LargeStringArray, ListArray, RunArray, StringArray, StringViewArray,
        };

        use super::cast_into;
        use yggdryl::cast::ArrowCastOptions;
        use yggdryl::{DataType, Field, Nullability, Scalar, Serie, TimeUnit, Timezone};

        /// A failed conversion is an error rather than a null.
        fn conversion_error() -> ArrowCastOptions {
            ArrowCastOptions::new().with_safe(false)
        }

        /// A required column refuses a null rather than repairing it.
        fn strict() -> ArrowCastOptions {
            ArrowCastOptions::new().with_nullability(Nullability::Strict)
        }

        const UNITS: [TimeUnit; 4] = [
            TimeUnit::Second,
            TimeUnit::Millisecond,
            TimeUnit::Microsecond,
            TimeUnit::Nanosecond,
        ];

        /// Every leaf that holds neither text nor bytes and reads a text column.
        fn non_text_targets() -> Vec<DataType> {
            let mut targets = vec![
                DataType::Int8,
                DataType::Int16,
                DataType::Int32,
                DataType::Int64,
                DataType::UInt8,
                DataType::UInt16,
                DataType::UInt32,
                DataType::UInt64,
                DataType::Float16,
                DataType::Float32,
                DataType::Float64,
                DataType::decimal32(9, 2).unwrap(),
                DataType::decimal64(18, 6).unwrap(),
                DataType::decimal128(38, 10).unwrap(),
                DataType::decimal256(76, 20).unwrap(),
                DataType::Boolean,
                DataType::date32(),
                DataType::date64(),
                DataType::time32(TimeUnit::Second).unwrap(),
                DataType::time32(TimeUnit::Millisecond).unwrap(),
                DataType::time64(TimeUnit::Microsecond).unwrap(),
                DataType::time64(TimeUnit::Nanosecond).unwrap(),
                DataType::uuid(),
                DataType::Version,
                DataType::url(),
                DataType::Timezone,
                DataType::MimeType,
                DataType::MediaType,
                DataType::geometry(None).unwrap(),
                DataType::geography(None, None).unwrap(),
                // An encoding is a layout: the values node answers.
                DataType::dictionary(DataType::Int8, DataType::Int32).unwrap(),
            ];
            targets.extend(codes_by_neutral_member().1);
            let zones: [Timezone; 4] = [
                Timezone::NAIVE,
                Timezone::UTC,
                "Europe/Paris".parse().unwrap(),
                "+05:30".parse().unwrap(),
            ];
            for unit in UNITS {
                for zone in zones {
                    targets.push(DataType::datetime64(unit, zone).unwrap());
                }
                targets.push(DataType::duration32(unit).unwrap());
                targets.push(DataType::duration64(unit).unwrap());
            }
            targets
        }

        /// The twelve registered codes.
        fn codes() -> Vec<DataType> {
            vec![
                DataType::Country,
                DataType::Ccy,
                DataType::MicCode,
                DataType::CfiCode,
                DataType::IsinCode,
                DataType::CusipCode,
                DataType::SedolCode,
                DataType::BloombergCode,
                DataType::FIGICode,
                DataType::Side,
                DataType::State,
                DataType::TimeInForce,
            ]
        }

        /// The codes holding the empty text as their neutral member - which is
        /// their canonical default - and the identifiers holding none, whose
        /// default is refused rather than invented.
        fn codes_by_neutral_member() -> (Vec<DataType>, Vec<DataType>) {
            codes()
                .into_iter()
                .partition(|code| code.default_value().is_ok())
        }

        /// The eighteen string leaves.
        fn string_leaves() -> Vec<DataType> {
            vec![
                DataType::utf8(),
                DataType::large_utf8(),
                DataType::utf8_view(),
                DataType::large_utf8_view(),
                DataType::fixed_utf8(3).unwrap(),
                DataType::sized_utf8(4).unwrap(),
                DataType::ascii(),
                DataType::large_ascii(),
                DataType::ascii_view(),
                DataType::large_ascii_view(),
                DataType::fixed_ascii(3).unwrap(),
                DataType::sized_ascii(4).unwrap(),
                DataType::cp1252(),
                DataType::large_cp1252(),
                DataType::cp1252_view(),
                DataType::large_cp1252_view(),
                DataType::fixed_cp1252(3).unwrap(),
                DataType::sized_cp1252(4).unwrap(),
            ]
        }

        /// The byte leaves that hold a payload of any length.
        fn byte_leaves() -> Vec<DataType> {
            vec![
                DataType::binary(),
                DataType::large_binary(),
                DataType::binary_view(),
                DataType::large_binary_view(),
                DataType::sized_binary(4).unwrap(),
            ]
        }

        /// One `""` cell under each plain text layout, and a dictionary and a
        /// run-end pair over one.
        fn empty_sources() -> Vec<ArrayRef> {
            vec![
                Arc::new(StringArray::from(vec![""])),
                Arc::new(LargeStringArray::from(vec![""])),
                Arc::new(StringViewArray::from(vec![""])),
                Arc::new(DictionaryArray::<Int8Type>::from_iter([Some("")])),
                Arc::new(
                    RunArray::<Int16Type>::try_new(
                        &Int16Array::from(vec![1]),
                        &StringArray::from(vec![""]),
                    )
                    .unwrap(),
                ),
            ]
        }

        fn cell(field: &Field, array: &ArrayRef) -> Scalar {
            Serie::from_arrow_array(Some(field), Arc::clone(array), ArrowCastOptions::default())
                .unwrap()
                .scalar(0)
                .unwrap()
        }

        /// Whether the one row is null as a reader sees it: a dictionary pair
        /// whose key points at a null value is null through its key.
        fn is_null(array: &ArrayRef) -> bool {
            array.logical_nulls().is_some_and(|nulls| nulls.is_null(0))
        }

        #[test]
        fn an_empty_cell_is_null_in_a_nullable_column_whatever_safe_says() {
            for target in non_text_targets() {
                let field = Field::new("x", target, true);
                for source in empty_sources() {
                    for options in [ArrowCastOptions::new(), conversion_error()] {
                        let cast = cast_into(&field, Arc::clone(&source), options).unwrap_or_else(
                            |error| {
                                panic!("{:?} -> {}: {error}", source.data_type(), field.dtype())
                            },
                        );
                        assert_eq!(cast.len(), 1, "{}", field.dtype());
                        assert!(
                            is_null(&cast),
                            "{:?} -> {} kept the empty cell",
                            source.data_type(),
                            field.dtype()
                        );
                    }
                }
            }
        }

        #[test]
        fn a_required_column_repairs_or_refuses_an_empty_cell_by_its_nullability() {
            for target in non_text_targets() {
                let field = Field::new("x", target, false);
                for source in empty_sources() {
                    let repaired = cast_into(&field, Arc::clone(&source), ArrowCastOptions::new());
                    match field.default_value() {
                        Ok(default) => {
                            let repaired = repaired.unwrap_or_else(|error| {
                                panic!("{:?} -> {}: {error}", source.data_type(), field.dtype())
                            });
                            assert_eq!(cell(&field, &repaired), default, "{}", field.dtype());
                        }
                        // A code with no neutral member has nothing to repair
                        // with, so the null the empty cell became is refused.
                        Err(_) => assert!(repaired.is_err(), "{}", field.dtype()),
                    }

                    let refused = cast_into(&field, Arc::clone(&source), strict())
                        .err()
                        .unwrap_or_else(|| {
                            panic!(
                                "{:?} -> {} took the empty cell",
                                source.data_type(),
                                field.dtype()
                            )
                        })
                        .to_string();
                    assert_eq!(
                        refused,
                        "required Arrow field $.x holds 1 null values",
                        "{:?} -> {}",
                        source.data_type(),
                        field.dtype()
                    );
                }
            }
        }

        #[test]
        fn a_spelling_no_reader_takes_keeps_its_own_answer_beside_an_empty_cell() {
            let field = Field::new("x", DataType::Int32, true);
            let mixed: ArrayRef = Arc::new(StringArray::from(vec!["7", "", "not a number"]));

            let lenient = cast_into(&field, Arc::clone(&mixed), ArrowCastOptions::new()).unwrap();
            let lenient = lenient.as_any().downcast_ref::<Int32Array>().unwrap();
            assert_eq!(lenient.value(0), 7);
            assert!(lenient.is_null(1));
            assert!(lenient.is_null(2));

            // The refusal is the misspelt cell's, and the empty one is never named.
            let refused = cast_into(&field, mixed, conversion_error())
                .unwrap_err()
                .to_string();
            assert!(refused.contains("not a number"), "{refused}");
            assert!(!refused.contains("''"), "{refused}");

            // Through one of the crate's own readers, whose wording names the
            // row: the misspelt cell is row 2, and row 1 - the empty one - is
            // never named.
            let dates = Field::new("x", DataType::date32(), true);
            let mixed: ArrayRef = Arc::new(StringArray::from(vec!["2024-01-01", "", "not a date"]));
            let refused = cast_into(&dates, mixed, conversion_error())
                .unwrap_err()
                .to_string();
            assert!(refused.contains("row 2"), "{refused}");
            assert!(!refused.contains("row 1"), "{refused}");

            // Whitespace is not empty: it is a spelling no reader takes.
            let blank: ArrayRef = Arc::new(StringArray::from(vec![" "]));
            let lenient = cast_into(&field, Arc::clone(&blank), ArrowCastOptions::new()).unwrap();
            assert!(lenient.is_null(0));
            assert!(cast_into(&field, blank, conversion_error()).is_err());
        }

        #[test]
        fn a_text_or_byte_column_keeps_an_empty_cell_as_the_value_it_is() {
            let empty: ArrayRef = Arc::new(StringArray::from(vec![""]));
            for leaf in string_leaves().into_iter().chain(byte_leaves()) {
                let field = Field::new("x", leaf, true);
                let cast = cast_into(&field, Arc::clone(&empty), conversion_error())
                    .unwrap_or_else(|error| panic!("{}: {error}", field.dtype()));
                assert!(!cast.is_null(0), "{}", field.dtype());
                assert_eq!(
                    cell(&field, &cast),
                    field.scalar("").unwrap(),
                    "{}",
                    field.dtype()
                );
            }

            // A fixed width is a rule about the payload, and an empty one is the
            // wrong width.
            assert!(
                cast_into(
                    &Field::new("x", DataType::fixed_binary(4).unwrap(), true),
                    empty,
                    conversion_error()
                )
                .is_err()
            );

            // The rule reads one direction: an empty payload renders as `""`.
            let payload: ArrayRef = Arc::new(BinaryArray::from_vec(vec![b""]));
            let field = Field::new("x", DataType::utf8(), true);
            let text = cast_into(&field, payload, conversion_error()).unwrap();
            assert!(!text.is_null(0));
            assert_eq!(cell(&field, &text), Scalar::from(""));
        }

        #[test]
        fn the_scalar_door_reads_an_empty_text_as_null_before_any_parser() {
            let empty = Scalar::from("");
            for target in [
                DataType::Int32,
                DataType::Float16,
                DataType::Float64,
                DataType::decimal128(10, 2).unwrap(),
                DataType::Boolean,
                DataType::date32(),
                DataType::date64(),
                DataType::time64(TimeUnit::Microsecond).unwrap(),
                DataType::datetime64(TimeUnit::Microsecond, Timezone::UTC).unwrap(),
                DataType::duration64(TimeUnit::Millisecond).unwrap(),
                DataType::uuid(),
                DataType::CusipCode,
                DataType::Version,
                DataType::url(),
                DataType::Timezone,
                DataType::MimeType,
                DataType::MediaType,
                DataType::geometry(None).unwrap(),
            ] {
                assert_eq!(target.scalar("").unwrap(), Scalar::Null, "{target}");
                assert_eq!(
                    target.cast_scalar(&empty).unwrap(),
                    Scalar::Null,
                    "{target}"
                );
                assert_eq!(target.try_cast_scalar(&empty), Scalar::Null, "{target}");

                let refused = Field::new("x", target.clone(), false)
                    .scalar("")
                    .unwrap_err()
                    .to_string();
                assert!(refused.contains("$.x"), "{target}: {refused}");
                assert!(refused.contains("null"), "{target}: {refused}");
            }

            assert_eq!(DataType::utf8().scalar("").unwrap(), Scalar::from(""));
            assert_eq!(
                DataType::binary().scalar("").unwrap().as_bytes(),
                Some(&[][..])
            );
        }

        /// A code with a neutral member holds the empty text as that member - it
        /// is the code's own canonical default - so a cast keeps it through every
        /// door and reads its own repair back; an identifier holds none, so the
        /// empty text is absence, as it is for a UUID.
        #[test]
        fn a_code_with_a_neutral_member_keeps_an_empty_cell_as_that_member() {
            let (neutral, identifiers) = codes_by_neutral_member();
            assert!(!neutral.is_empty());
            assert!(!identifiers.is_empty());
            for code in neutral {
                let member = code.default_value().unwrap();
                assert_eq!(code.scalar("").unwrap(), member, "{code}");
                assert_eq!(
                    code.cast_scalar(&Scalar::from("")).unwrap(),
                    member,
                    "{code}"
                );
                for nullable in [true, false] {
                    let field = Field::new("x", code.clone(), nullable);
                    for source in empty_sources() {
                        for options in [ArrowCastOptions::new(), conversion_error(), strict()] {
                            let cast = cast_into(&field, Arc::clone(&source), options)
                                .unwrap_or_else(|error| {
                                    panic!("{:?} -> {}: {error}", source.data_type(), field.dtype())
                                });
                            assert!(!cast.is_null(0), "{:?} -> {code}", source.data_type());
                            assert_eq!(cell(&field, &cast), member, "{code}");
                        }
                    }
                }

                // Idempotence: what a required column holds after its own
                // repair, and its own default array, read back under Strict.
                let required = Field::new("x", code.clone(), false);
                for column in [
                    cast_into(
                        &required,
                        Arc::new(StringArray::from(vec![""])),
                        ArrowCastOptions::new(),
                    )
                    .unwrap(),
                    Serie::from_default(required.clone(), 1)
                        .unwrap()
                        .require_arrow_array()
                        .unwrap(),
                ] {
                    let again = cast_into(&required, column, strict())
                        .unwrap_or_else(|error| panic!("{code}: {error}"));
                    assert_eq!(cell(&required, &again), member, "{code}");
                }
            }
            for code in identifiers {
                assert_eq!(code.scalar("").unwrap(), Scalar::Null, "{code}");
            }
        }

        /// An interval has no text spelling, so an empty one is a spelling it
        /// refuses rather than absence: today's answer, through both doors.
        #[test]
        fn an_interval_refuses_an_empty_cell_as_the_spelling_it_is_not() {
            let interval = DataType::interval(TimeUnit::DayTime).unwrap();
            assert!(interval.scalar("").is_err());
            assert!(interval.cast_scalar(&Scalar::from("")).is_err());
            assert_eq!(interval.try_cast_scalar(&Scalar::from("")), Scalar::Null);

            let field = Field::new("x", interval, true);
            let empty: ArrayRef = Arc::new(StringArray::from(vec![""]));
            let lenient = cast_into(&field, Arc::clone(&empty), ArrowCastOptions::new()).unwrap();
            assert!(lenient.is_null(0));
            assert!(cast_into(&field, empty, conversion_error()).is_err());
        }

        /// A serie target reads a scalar source into its item, so the item is
        /// what answers: a text item keeps the empty cell, a numeric one does not.
        #[test]
        fn a_serie_target_answers_for_its_item() {
            let empty: ArrayRef = Arc::new(StringArray::from(vec![""]));

            let texts = Field::new(
                "x",
                DataType::serie(DataType::utf8().nullable_field("item")),
                true,
            );
            let cast = cast_into(&texts, Arc::clone(&empty), conversion_error()).unwrap();
            let list = cast.as_any().downcast_ref::<ListArray>().unwrap();
            assert_eq!(list.value_length(0), 1);
            assert!(!list.values().is_null(0));
            assert_eq!(
                list.values()
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .unwrap()
                    .value(0),
                ""
            );

            let counts = Field::new(
                "x",
                DataType::serie(DataType::Int32.nullable_field("item")),
                true,
            );
            let cast = cast_into(&counts, empty, conversion_error()).unwrap();
            let list = cast.as_any().downcast_ref::<ListArray>().unwrap();
            assert_eq!(list.value_length(0), 1);
            assert!(list.values().is_null(0));
        }
    }
}

mod strict {
    use std::sync::Arc;

    use arrow_array::builder::{Int32Builder, ListBuilder, MapBuilder, StringBuilder};
    use arrow_array::types::Int16Type;
    use arrow_array::{
        Array, ArrayRef, DictionaryArray, Int32Array, Int64Array, RecordBatch, StringArray,
        StructArray,
    };
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Fields, Schema, SchemaRef};
    use yggdryl::{
        ArrowCastOptions, ArrowCastPlan, DataType, Field, Nullability, Serie, StructType,
    };

    fn root(fields: impl IntoIterator<Item = Field>) -> Field {
        Field::new(
            "row",
            DataType::from(StructType::from_fields(fields).unwrap()),
            false,
        )
    }

    fn strict() -> ArrowCastOptions {
        ArrowCastOptions::new().with_nullability(Nullability::Strict)
    }

    fn schema(fields: Vec<ArrowField>) -> SchemaRef {
        Arc::new(Schema::new(fields))
    }

    /// `batch` cast into `target`, as the table of the column that comes out.
    fn cast_batch(
        target: &Field,
        batch: &RecordBatch,
        options: ArrowCastOptions,
    ) -> yggdryl::arrow::Result<RecordBatch> {
        Serie::from_arrow_batch(Some(target), batch, options)?.into_arrow_batch()
    }

    fn refusal(target: &Field, batch: RecordBatch) -> String {
        cast_batch(target, &batch, strict())
            .unwrap_err()
            .to_string()
    }

    #[test]
    fn a_required_column_the_source_does_not_carry_is_refused_by_path() {
        let source = schema(vec![ArrowField::new("id", ArrowDataType::Int32, false)]);
        let batch = RecordBatch::try_new(
            Arc::clone(&source),
            vec![Arc::new(Int32Array::from(vec![1]))],
        )
        .unwrap();
        let target = root([
            DataType::Int64.required_field("id"),
            DataType::utf8().required_field("symbol"),
        ]);

        // The default policy is unchanged: the hole is filled, not reported.
        let filled = cast_batch(&target, &batch, ArrowCastOptions::new()).unwrap();
        assert_eq!(filled.column(1).null_count(), 0);
        assert_eq!(filled.num_columns(), 2);

        assert_eq!(
            refusal(&target, batch),
            "required Arrow field $.symbol is missing from the source"
        );

        // The schemas alone decide it, so compiling is where it fails - a reader
        // never pulls a batch to find out.
        let source = Field::from_arrow_schema("row", &source).unwrap();
        let message = ArrowCastPlan::compile(&source, &target, strict())
            .unwrap_err()
            .to_string();
        assert_eq!(
            message,
            "required Arrow field $.symbol is missing from the source"
        );
    }

    #[test]
    fn a_null_in_a_required_column_is_refused_with_its_count() {
        let source = schema(vec![
            ArrowField::new("id", ArrowDataType::Int64, false),
            ArrowField::new("symbol", ArrowDataType::Utf8, true),
        ]);
        let batch = RecordBatch::try_new(
            source,
            vec![
                Arc::new(Int64Array::from(vec![1, 2, 3])),
                Arc::new(StringArray::from(vec![Some("AAPL"), None, None])),
            ],
        )
        .unwrap();
        let target = root([
            DataType::Int64.required_field("id"),
            DataType::utf8().required_field("symbol"),
        ]);

        let filled = cast_batch(&target, &batch, ArrowCastOptions::new()).unwrap();
        assert_eq!(filled.column(1).null_count(), 0);

        assert_eq!(
            refusal(&target, batch),
            "required Arrow field $.symbol holds 2 null values"
        );
    }

    #[test]
    fn a_missing_nullable_column_stays_all_null_under_both_policies() {
        let source = schema(vec![ArrowField::new("id", ArrowDataType::Int64, false)]);
        let batch =
            RecordBatch::try_new(source, vec![Arc::new(Int64Array::from(vec![1, 2]))]).unwrap();
        let target = root([
            DataType::Int64.required_field("id"),
            DataType::utf8().nullable_field("symbol"),
        ]);

        for options in [ArrowCastOptions::new(), strict()] {
            let cast = cast_batch(&target, &batch, options).unwrap();
            assert_eq!(cast.num_columns(), 2);
            assert_eq!(cast.column(1).null_count(), 2);
        }
    }

    #[test]
    fn an_undeclared_source_column_stays_dropped_under_both_policies() {
        let source = schema(vec![
            ArrowField::new("id", ArrowDataType::Int64, false),
            ArrowField::new("unused", ArrowDataType::Int32, true),
        ]);
        let batch = RecordBatch::try_new(
            source,
            vec![
                Arc::new(Int64Array::from(vec![1])),
                Arc::new(Int32Array::from(vec![Some(7)])),
            ],
        )
        .unwrap();
        let target = root([DataType::Int64.required_field("id")]);

        for options in [ArrowCastOptions::new(), strict()] {
            let cast = cast_batch(&target, &batch, options).unwrap();
            assert_eq!(cast.schema().fields().len(), 1);
            assert_eq!(cast.schema().field(0).name(), "id");
        }
    }

    #[test]
    fn a_nested_struct_child_is_named_by_its_whole_path() {
        let address = ArrowDataType::Struct(Fields::from(vec![ArrowField::new(
            "city",
            ArrowDataType::Utf8,
            true,
        )]));
        let source = schema(vec![ArrowField::new("address", address.clone(), false)]);
        let batch = RecordBatch::try_new(
            source,
            vec![Arc::new(StructArray::new(
                Fields::from(vec![ArrowField::new("city", ArrowDataType::Utf8, true)]),
                vec![Arc::new(StringArray::from(vec!["Paris"])) as ArrayRef],
                None,
            ))],
        )
        .unwrap();

        // A required child the source struct does not carry at all.
        let missing = root([StructType::from_fields([
            DataType::utf8().nullable_field("city"),
            DataType::utf8().required_field("zip code"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("address")]);
        assert_eq!(
            refusal(&missing, batch.clone()),
            "required Arrow field $.address[\"zip code\"] is missing from the source"
        );

        // A required child the source carries, holding null.
        let null_source = schema(vec![ArrowField::new("address", address, false)]);
        let null_batch = RecordBatch::try_new(
            null_source,
            vec![Arc::new(StructArray::new(
                Fields::from(vec![ArrowField::new("city", ArrowDataType::Utf8, true)]),
                vec![Arc::new(StringArray::from(vec![None::<&str>])) as ArrayRef],
                None,
            ))],
        )
        .unwrap();
        let required_child = root([StructType::from_fields([
            DataType::utf8().required_field("city")
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("address")]);
        assert_eq!(
            refusal(&required_child, null_batch),
            "required Arrow field $.address.city holds 1 null values"
        );
    }

    #[test]
    fn a_required_serie_item_is_named_under_its_serie() {
        let mut builder = ListBuilder::new(Int32Builder::new());
        builder.values().append_value(1);
        builder.values().append_null();
        builder.append(true);
        let values: ArrayRef = Arc::new(builder.finish());
        let source = schema(vec![ArrowField::new(
            "counts",
            values.data_type().clone(),
            true,
        )]);
        let batch = RecordBatch::try_new(source, vec![values]).unwrap();

        let target = root([
            DataType::Serie(Arc::new(DataType::Int32.required_field("item")))
                .nullable_field("counts"),
        ]);
        assert_eq!(
            refusal(&target, batch),
            "required Arrow field $.counts[] holds 1 null values"
        );
    }

    #[test]
    fn a_required_map_value_is_named_under_its_entries() {
        let mut builder = MapBuilder::new(None, StringBuilder::new(), Int32Builder::new());
        builder.keys().append_value("a");
        builder.values().append_null();
        builder.append(true).unwrap();
        let map: ArrayRef = Arc::new(builder.finish());
        let source = schema(vec![ArrowField::new("tags", map.data_type().clone(), true)]);
        let batch = RecordBatch::try_new(source, vec![map]).unwrap();

        let entries = StructType::from_fields([
            DataType::utf8().required_field("keys"),
            DataType::Int32.required_field("values"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("entries");
        let target = root([DataType::map(entries, false)
            .unwrap()
            .nullable_field("tags")]);
        assert_eq!(
            refusal(&target, batch),
            "required Arrow field $.tags.entries.values holds 1 null values"
        );
    }

    #[test]
    fn a_dictionary_refuses_a_null_its_values_reach() {
        let dictionary: ArrayRef = Arc::new(DictionaryArray::<Int16Type>::from_iter([
            Some("alpha"),
            None,
        ]));
        let source = schema(vec![ArrowField::new(
            "label",
            dictionary.data_type().clone(),
            true,
        )]);
        let batch = RecordBatch::try_new(source, vec![dictionary]).unwrap();

        let target = root([DataType::dictionary(DataType::Int16, DataType::utf8())
            .unwrap()
            .required_field("label")]);
        assert_eq!(
            refusal(&target, batch),
            "required Arrow field $.label holds 1 null values"
        );
    }

    #[test]
    fn strictness_and_conversion_safety_answer_different_questions() {
        let source = schema(vec![ArrowField::new("id", ArrowDataType::Utf8, false)]);
        let batch = RecordBatch::try_new(
            source,
            vec![Arc::new(StringArray::from(vec!["1", "not a number"]))],
        )
        .unwrap();
        let target = root([DataType::Int64.required_field("id")]);

        // safe: the failed conversion becomes null, and the default policy
        // repairs that null in a required column.
        let repaired = cast_batch(&target, &batch, ArrowCastOptions::new()).unwrap();
        assert_eq!(repaired.column(0).null_count(), 0);
        // Strictness refuses the null a lenient conversion would leave in a
        // required column, so the conversion is refused by the value itself
        // rather than by the null it would have become.
        let strict_message = refusal(&target, batch.clone());
        assert!(strict_message.contains("not a number"), "{strict_message}");
        // A nullable column still takes the failed conversion as null.
        let nullable = root([DataType::Int64.nullable_field("id")]);
        let nulled = cast_batch(&nullable, &batch, strict()).unwrap();
        assert_eq!(nulled.column(0).null_count(), 1);

        // Unsafe: the conversion itself refuses, so strictness never sees a null.
        let unsafe_message = cast_batch(&target, &batch, strict().with_safe(false))
            .unwrap_err()
            .to_string();
        assert!(unsafe_message.contains("not a number"), "{unsafe_message}");

        // An empty cell is not a failed conversion: it is null before `safe` is
        // asked, so an unsafe cast passes it into a nullable column as one.
        let source = schema(vec![ArrowField::new("id", ArrowDataType::Utf8, false)]);
        let empty =
            RecordBatch::try_new(source, vec![Arc::new(StringArray::from(vec!["1", ""]))]).unwrap();
        let nullable = root([DataType::Int64.nullable_field("id")]);
        let passed = cast_batch(&nullable, &empty, strict().with_safe(false)).unwrap();
        assert_eq!(passed.column(0).null_count(), 1);
        assert!(passed.column(0).is_null(1));
    }

    /// An empty text cell under a required declaration is a null under it: the
    /// default policy repairs it with the child's default and the strict one
    /// refuses it by the whole path, exactly as it does for a null the source
    /// carried.
    #[test]
    fn an_empty_text_cell_in_a_required_child_is_a_null_by_its_path() {
        // A struct child.
        let city = Fields::from(vec![ArrowField::new("zip", ArrowDataType::Utf8, true)]);
        let source = schema(vec![ArrowField::new(
            "address",
            ArrowDataType::Struct(city.clone()),
            false,
        )]);
        let batch = RecordBatch::try_new(
            source,
            vec![Arc::new(StructArray::new(
                city,
                vec![Arc::new(StringArray::from(vec![""])) as ArrayRef],
                None,
            ))],
        )
        .unwrap();
        let target = root([
            StructType::from_fields([DataType::Int32.required_field("zip")])
                .map(DataType::from)
                .unwrap()
                .required_field("address"),
        ]);
        let repaired = cast_batch(&target, &batch, ArrowCastOptions::new()).unwrap();
        let address = repaired
            .column(0)
            .as_any()
            .downcast_ref::<StructArray>()
            .unwrap();
        assert_eq!(address.column(0).null_count(), 0);
        assert_eq!(
            address
                .column(0)
                .as_any()
                .downcast_ref::<Int32Array>()
                .unwrap()
                .value(0),
            0
        );
        assert_eq!(
            refusal(&target, batch),
            "required Arrow field $.address.zip holds 1 null values"
        );

        // A serie item.
        let mut builder = ListBuilder::new(StringBuilder::new());
        builder.values().append_value("7");
        builder.values().append_value("");
        builder.append(true);
        let values: ArrayRef = Arc::new(builder.finish());
        let source = schema(vec![ArrowField::new(
            "counts",
            values.data_type().clone(),
            true,
        )]);
        let batch = RecordBatch::try_new(source, vec![values]).unwrap();
        let target = root([
            DataType::Serie(Arc::new(DataType::Int32.required_field("item")))
                .nullable_field("counts"),
        ]);
        let repaired = cast_batch(&target, &batch, ArrowCastOptions::new()).unwrap();
        let counts = repaired
            .column(0)
            .as_any()
            .downcast_ref::<arrow_array::ListArray>()
            .unwrap();
        assert_eq!(counts.values().null_count(), 0);
        let items = counts
            .values()
            .as_any()
            .downcast_ref::<Int32Array>()
            .unwrap();
        assert_eq!((items.value(0), items.value(1)), (7, 0));
        assert_eq!(
            refusal(&target, batch),
            "required Arrow field $.counts[] holds 1 null values"
        );

        // A map value.
        let mut builder = MapBuilder::new(None, StringBuilder::new(), StringBuilder::new());
        builder.keys().append_value("a");
        builder.values().append_value("");
        builder.append(true).unwrap();
        let map: ArrayRef = Arc::new(builder.finish());
        let source = schema(vec![ArrowField::new("tags", map.data_type().clone(), true)]);
        let batch = RecordBatch::try_new(source, vec![map]).unwrap();
        let entries = StructType::from_fields([
            DataType::utf8().required_field("keys"),
            DataType::Int32.required_field("values"),
        ])
        .map(DataType::from)
        .unwrap()
        .required_field("entries");
        let target = root([DataType::map(entries, false)
            .unwrap()
            .nullable_field("tags")]);
        let repaired = cast_batch(&target, &batch, ArrowCastOptions::new()).unwrap();
        let tags = repaired
            .column(0)
            .as_any()
            .downcast_ref::<arrow_array::MapArray>()
            .unwrap();
        assert_eq!(tags.values().null_count(), 0);
        assert_eq!(
            tags.values()
                .as_any()
                .downcast_ref::<Int32Array>()
                .unwrap()
                .value(0),
            0
        );
        assert_eq!(
            refusal(&target, batch),
            "required Arrow field $.tags.entries.values holds 1 null values"
        );
    }

    /// A reader that takes only a plain text layout answers a source carrying no
    /// visible value with a null column, so the null policy - never the reader's
    /// own default - decides for a required target: repaired under the default
    /// policy, refused by path under the strict one, for a source null exactly as
    /// for an empty cell.
    #[test]
    fn a_deferred_reader_hands_an_all_null_column_to_the_null_policy() {
        let column: ArrayRef = Arc::new(DictionaryArray::<Int16Type>::from_iter([None::<&str>]));
        let source = schema(vec![ArrowField::new(
            "release",
            column.data_type().clone(),
            true,
        )]);
        let batch = RecordBatch::try_new(source, vec![column]).unwrap();
        let release = DataType::Version.required_field("release");
        let target = root([release.clone()]);

        let repaired = cast_batch(&target, &batch, ArrowCastOptions::new()).unwrap();
        assert_eq!(repaired.column(0).null_count(), 0);
        assert_eq!(
            Serie::from_arrow_array(
                Some(&release),
                Arc::clone(repaired.column(0)),
                ArrowCastOptions::default()
            )
            .unwrap()
            .scalar(0)
            .unwrap(),
            release.default_value().unwrap()
        );
        assert_eq!(
            refusal(&target, batch),
            "required Arrow field $.release holds 1 null values"
        );
    }

    #[test]
    fn extension_and_schema_metadata_survive_a_strict_cast() {
        let source = schema(vec![ArrowField::new("id", ArrowDataType::Int32, false)]);
        let batch =
            RecordBatch::try_new(source, vec![Arc::new(Int32Array::from(vec![1]))]).unwrap();

        let mut identifier = DataType::uuid().nullable_field("identifier");
        identifier.set_metadata([("owner", "trading")]).unwrap();
        let mut target = root([DataType::Int64.required_field("id"), identifier]);
        target.set_metadata([("source", "book")]).unwrap();

        let cast = cast_batch(&target, &batch, strict()).unwrap();
        let cast_schema = cast.schema();
        assert_eq!(
            cast_schema.metadata().get("source").map(String::as_str),
            Some("book")
        );
        let projected = cast_schema.field(1);
        assert_eq!(
            projected.metadata().get("owner").map(String::as_str),
            Some("trading")
        );
        assert_eq!(
            projected.extension_type_name(),
            Some("arrow.uuid"),
            "{projected:?}"
        );
    }

    #[test]
    fn an_ambiguous_case_insensitive_name_is_refused_before_any_batch() {
        let source = schema(vec![
            ArrowField::new("id", ArrowDataType::Int64, false),
            ArrowField::new("ID", ArrowDataType::Int64, false),
        ]);
        let target = root([DataType::Int64.required_field("id")]);

        let source = Field::from_arrow_schema("row", &source).unwrap();
        let message = ArrowCastPlan::compile(&source, &target, ArrowCastOptions::new())
            .unwrap_err()
            .to_string();
        assert!(message.contains("ambiguous"), "{message}");
    }
}

/// Every ingest the plan certifies writes only values its target's own
/// contract accepts, so a column it lands is never read a second time and
/// never holds a row [`Field::scalar`] would refuse.
mod certification {
    use std::sync::Arc;

    use arrow_array::{ArrayRef, BinaryArray, FixedSizeBinaryArray, StringArray};
    use yggdryl::{ArrowCastOptions, DataType, Field, Serie};

    /// Cast `source` into a nullable `target` under the default options - the
    /// certified path - and hold every landed row to the target's contract.
    fn certified(target: DataType, source: ArrayRef) {
        let field = Field::new("value", target, true);
        // Under the default options a value the target refuses is nulled, so
        // every input lands and every landed row is held to the contract.
        let serie = Serie::from_arrow_array(Some(&field), source, ArrowCastOptions::new())
            .unwrap_or_else(|error| panic!("{field}: the certified cast lands: {error}"));
        assert!(!serie.is_empty());
        for index in 0..serie.len() {
            let value = serie.scalar(index).expect("a landed row reads");
            assert!(
                field.scalar(value.clone()).is_ok(),
                "{field}: row {index} {value:?} landed certified but its field refuses it"
            );
        }
    }

    fn text(values: &[Option<&str>]) -> ArrayRef {
        Arc::new(StringArray::from(values.to_vec()))
    }

    #[test]
    fn the_string_ingest_writes_only_what_the_string_accepts() {
        let spellings = text(&[
            Some("ab"),
            Some("abcdef"),
            Some(""),
            None,
            Some("é"),
            Some("€"),
            Some("中"),
        ]);
        for target in [
            DataType::sized_utf8(4).unwrap(),
            DataType::fixed_ascii(3).unwrap(),
            DataType::sized_cp1252(4).unwrap(),
            DataType::fixed_cp1252(2).unwrap(),
        ] {
            certified(target, Arc::clone(&spellings));
        }
        let bytes: ArrayRef = Arc::new(BinaryArray::from(vec![
            Some(b"ok".as_ref()),
            Some(b"\xff\xfe".as_ref()),
            Some(b"\x80".as_ref()),
        ]));
        certified(DataType::utf8(), Arc::clone(&bytes));
        certified(DataType::sized_cp1252(2).unwrap(), bytes);
    }

    #[test]
    fn the_bytes_ingest_writes_only_what_the_bound_accepts() {
        let bytes: ArrayRef = Arc::new(BinaryArray::from(vec![
            Some(b"ab".as_ref()),
            Some(b"abcd".as_ref()),
            None,
        ]));
        certified(DataType::sized_binary(3).unwrap(), bytes);
    }

    #[test]
    fn the_code_ingest_writes_only_registered_members() {
        let codes = text(&[Some("USD"), Some("usd"), Some("EURO"), Some(""), None]);
        for target in [
            DataType::Ccy,
            DataType::Country,
            DataType::Side,
            DataType::State,
        ] {
            certified(target, Arc::clone(&codes));
        }
    }

    #[test]
    fn the_uuid_url_and_urn_ingests_write_only_what_they_parse() {
        // The UUID ingest refuses a spelling it cannot read even under
        // `safe`, so only spellings that land are held to the contract here.
        let uuids = text(&[
            Some("67e55044-10b1-426f-9247-bb680e5fe0c8"),
            Some("67E5504410B1426F9247BB680E5FE0C8"),
            None,
        ]);
        certified(DataType::Uuid, uuids);
        let raw: ArrayRef = Arc::new(
            FixedSizeBinaryArray::try_from_iter([[7_u8; 16], [0_u8; 16]].into_iter()).unwrap(),
        );
        certified(DataType::Uuid, raw);
        // The URL and URN ingests refuse too, and rewrite what they read to
        // its canonical text: that rewrite is what is held to the contract.
        let urls = text(&[Some("HTTPS://Example.com/a"), Some(""), None]);
        certified(DataType::Url, urls);
        let urns = text(&[Some("URN:isbn:0451450523"), None]);
        certified(DataType::Urn, urns);
    }
}

mod chunked {
    //! One plan over every chunk of a chunked column, the chunks kept apart.

    use yggdryl::{ArrowCastOptions, ArrowCastPlan, ChunkedSerie, DataType, Field, Scalar, Serie};

    fn prices() -> (Field, ChunkedSerie) {
        let price = Field::new("price", DataType::Int32, false);
        let column = |rows: &[i32]| {
            Serie::from_scalars(price.clone(), rows.iter().copied().map(Scalar::from))
                .expect("int32 rows")
        };
        let chunked = ChunkedSerie::from_series(
            Some(&price),
            [column(&[125, 126]), column(&[127])],
            ArrowCastOptions::new(),
        )
        .expect("two chunks");
        (price, chunked)
    }

    #[test]
    fn a_plan_casts_every_chunk_and_keeps_them_apart() {
        let (price, chunked) = prices();
        let wide = Field::new("price", DataType::Int64, false);
        let plan = ArrowCastPlan::compile(&price, &wide, ArrowCastOptions::new()).unwrap();
        let cast = plan.apply_chunked(&chunked).unwrap();
        assert_eq!((cast.num_chunks(), cast.field()), (2, &wide));
        assert_eq!(cast.scalar(2).unwrap(), Scalar::from(127_i64));
        assert!(cast == chunked);
    }

    #[test]
    fn an_identity_plan_hands_the_chunked_column_back_and_a_foreign_layout_is_refused() {
        let (price, chunked) = prices();
        let same = ArrowCastPlan::compile(&price, &price, ArrowCastOptions::new()).unwrap();
        let back = same.apply_chunked(&chunked).unwrap();
        assert!(std::sync::Arc::ptr_eq(
            back.field_ref(),
            chunked.field_ref()
        ));
        assert_eq!(back.num_chunks(), 2);

        let text = ChunkedSerie::from_serie(
            Serie::from_scalars(
                Field::new("price", DataType::utf8(), false),
                [Scalar::from("1")],
            )
            .unwrap(),
        )
        .unwrap();
        let refusal = same.apply_chunked(&text).unwrap_err();
        assert!(refusal.to_string().contains("compiled for"), "{refusal}");

        // No chunk is still judged by its field, and answered under the target.
        let wide = Field::new("price", DataType::Int64, true);
        let plan = ArrowCastPlan::compile(&price, &wide, ArrowCastOptions::new()).unwrap();
        let empty = plan
            .apply_chunked(&ChunkedSerie::empty(price.clone()).unwrap())
            .unwrap();
        assert_eq!((empty.num_chunks(), empty.field()), (0, &wide));
        assert!(plan.apply_chunked(&text).is_err());
    }
}
