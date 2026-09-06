//! What the generic Arrow scalar family holds, and what it narrows to.

use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Datum, Int64Array, RecordBatch, StringArray, StructArray};

use super::{ArrowShape, ArrowValue};
use crate::arrow::batch_reader;
use crate::{ArrowCastOptions, DataType, Field};

fn quote_root() -> Field {
    DataType::from_fields([
        DataType::Utf8.required_field("symbol"),
        DataType::Int64.required_field("size"),
    ])
    .expect("the root datatype is valid")
    .required_field("row")
}

fn quote_batch() -> RecordBatch {
    let schema = quote_root()
        .into_arrow_schema()
        .expect("the root projects to Arrow");
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec!["AAPL", "MSFT"])) as ArrayRef,
            Arc::new(Int64Array::from(vec![100_i64, 250])),
        ],
    )
    .expect("the columns match the schema")
}

fn prices() -> ArrayRef {
    Arc::new(Int64Array::from(vec![125_i64, 126, 127]))
}

mod shapes {
    use super::{ArrowShape, ArrowValue, Field, prices, quote_batch, quote_root};
    use crate::{DataType, Scalar};

    #[test]
    fn every_shape_reports_itself_and_its_width() {
        let field = Field::new("price", DataType::Int64, false);
        let scalar = ArrowValue::from_value(&field, &Scalar::from(125_i64))
            .expect("one value materializes");
        let array = ArrowValue::from_array(field, prices()).expect("the column pairs");
        let batch = ArrowValue::from_batch(quote_batch()).expect("the batch names its root");

        assert_eq!(scalar.shape(), ArrowShape::Scalar);
        assert_eq!(scalar.row_size(), Some(1));
        assert_eq!(scalar.column_size(), 1);

        assert_eq!(array.shape(), ArrowShape::Array);
        assert_eq!(array.row_size(), Some(3));

        assert_eq!(batch.shape(), ArrowShape::Batch);
        assert_eq!(batch.row_size(), Some(2));
        assert_eq!(batch.column_size(), 2);
        assert_eq!(batch.field().name(), "row");
    }

    #[test]
    fn a_stream_states_its_root_without_being_pulled() {
        let reader = crate::arrow::batch_reader(
            quote_root()
                .into_arrow_schema()
                .expect("the root projects to Arrow"),
            [quote_batch()],
        );
        let value = ArrowValue::from_reader(reader).expect("the reader names its root");

        // Nothing was read: a stream has no length until it is drained.
        assert_eq!(value.shape(), ArrowShape::Stream);
        assert_eq!(value.row_size(), None);
        assert_eq!(value.column_size(), 2);
        assert!(value.shape().is_streamed());
        assert!(value.shape().is_tabular());
    }

    #[test]
    fn shape_names_round_trip_through_their_canonical_spelling() {
        for shape in ArrowShape::ALL {
            assert_eq!(
                ArrowShape::from_str(shape.as_str()).expect("a canonical name parses"),
                shape
            );
            assert_eq!(shape.to_string(), shape.as_str());
        }
        assert!(ArrowShape::from_str("table").is_err());
    }
}

mod pairing {
    use super::{ArrowValue, Field, prices, quote_batch, quote_root};
    use crate::{DataType, Scalar};

    #[test]
    fn a_scalar_holds_exactly_one_row() {
        let field = Field::new("price", DataType::Int64, false);
        assert!(ArrowValue::from_scalar_array(field.clone(), prices()).is_err());

        let one = prices().slice(0, 1);
        assert!(ArrowValue::from_scalar_array(field, one).is_ok());
    }

    #[test]
    fn a_column_must_carry_the_declared_layout() {
        // int64 values under a utf8 field are refused at the pairing, not at
        // the first read.
        let wrong = Field::new("price", DataType::Utf8, false);
        assert!(ArrowValue::from_array(wrong, prices()).is_err());
    }

    #[test]
    fn a_declared_root_must_be_exactly_the_rows_own_schema() {
        let widened = DataType::from_fields([
            DataType::Utf8.required_field("symbol"),
            DataType::Int64.nullable_field("size"),
        ])
        .expect("the root datatype is valid")
        .required_field("row");

        // Nullability is part of the schema, so this is a mismatch and not a
        // silent conversion.
        assert!(ArrowValue::from_batch_as(widened, quote_batch()).is_err());
        assert!(ArrowValue::from_batch_as(quote_root(), quote_batch()).is_ok());
    }

    #[test]
    fn native_rows_cross_through_the_one_value_boundary() {
        let rows = Scalar::from_sequence([
            Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from(100_i64)]),
            Scalar::from_sequence([Scalar::from("MSFT"), Scalar::from(250_i64)]),
        ]);
        let value = ArrowValue::from_rows(&quote_root(), &rows).expect("the rows materialize");

        assert_eq!(value.row_size(), Some(2));
        assert_eq!(value.into_scalar().expect("the rows decode"), rows);
    }
}

mod narrowing {
    use super::{
        Array, ArrowValue, Datum, Field, StructArray, batch_reader, prices, quote_batch,
        quote_root,
    };
    use crate::{DataType, Scalar};

    #[test]
    fn every_shape_widens_to_the_one_reader_a_record_write_takes() {
        let field = Field::new("price", DataType::Int64, false);
        let cases = [
            ArrowValue::from_array(field, prices()).expect("the column pairs"),
            ArrowValue::from_batch(quote_batch()).expect("the batch names its root"),
        ];
        let expected = [3, 2];
        for (value, rows) in cases.into_iter().zip(expected) {
            let reader = value.into_reader().expect("the value widens");
            assert_eq!(
                reader
                    .map(|batch| batch.expect("a batch reads").num_rows())
                    .sum::<usize>(),
                rows
            );
        }
    }

    #[test]
    fn a_bare_column_becomes_the_single_column_of_a_default_root() {
        let field = Field::new("price", DataType::Int64, false);
        let value = ArrowValue::from_array(field, prices()).expect("the column pairs");

        let batch = value.into_batch().expect("the column lays out as rows");
        assert_eq!(batch.num_columns(), 1);
        assert_eq!(batch.num_rows(), 3);
        assert_eq!(batch.schema().field(0).name(), "price");
    }

    #[test]
    fn a_batch_narrows_to_the_struct_column_its_rows_already_are() {
        let value = ArrowValue::from_batch(quote_batch()).expect("the batch names its root");
        let array = value.into_array().expect("the rows narrow");
        let structs = array
            .as_any()
            .downcast_ref::<StructArray>()
            .expect("rows are a struct column");

        assert_eq!(structs.len(), 2);
        assert_eq!(structs.num_columns(), 2);
    }

    #[test]
    fn draining_a_stream_concatenates_every_batch_it_yielded() {
        let schema = quote_root()
            .into_arrow_schema()
            .expect("the root projects to Arrow");
        let reader = batch_reader(schema, [quote_batch(), quote_batch()]);
        let value = ArrowValue::from_reader(reader).expect("the reader names its root");

        assert_eq!(value.into_batch().expect("the stream drains").num_rows(), 4);
    }

    #[test]
    fn only_a_one_row_value_becomes_an_arrow_datum() {
        let field = Field::new("price", DataType::Int64, false);
        let many = ArrowValue::from_array(field.clone(), prices()).expect("the column pairs");
        assert!(many.into_arrow_scalar().is_err());

        let one = ArrowValue::from_value(&field, &Scalar::from(125_i64))
            .expect("one value materializes");
        assert_eq!(
            one.into_arrow_scalar()
                .expect("one row is a datum")
                .get()
                .0
                .len(),
            1
        );
    }

    #[test]
    fn a_stream_of_rows_decodes_as_one_sequence() {
        let schema = quote_root()
            .into_arrow_schema()
            .expect("the root projects to Arrow");
        let reader = batch_reader(schema, [quote_batch()]);
        let value = ArrowValue::from_reader(reader).expect("the reader names its root");
        let rows = value.into_scalar().expect("the stream decodes");

        assert_eq!(rows.as_sequence().map(<[Scalar]>::len), Some(2));
    }
}

mod casting {
    use super::{ArrowCastOptions, ArrowValue, Field, batch_reader, prices};
    use crate::{DataType, Scalar};

    #[test]
    fn a_column_is_reshaped_and_keeps_its_shape() {
        let source = Field::new("price", DataType::Int64, false);
        let target = Field::new("price", DataType::Float64, false);
        let value = ArrowValue::from_array(source, prices()).expect("the column pairs");

        let cast = value
            .cast(&target, ArrowCastOptions::new())
            .expect("int64 widens to float64");
        assert!(cast.is_array());
        assert_eq!(cast.field().dtype(), &DataType::Float64);
        assert_eq!(cast.row_size(), Some(3));
    }

    #[test]
    fn a_stream_is_cast_one_batch_at_a_time_under_one_plan() {
        let source = DataType::from_fields([DataType::Int64.required_field("size")])
            .expect("the root datatype is valid")
            .required_field("row");
        let target = DataType::from_fields([DataType::Float64.required_field("size")])
            .expect("the root datatype is valid")
            .required_field("row");
        let batch = crate::arrow::batch_from_value(
            &source,
            &Scalar::from_sequence([Scalar::from_sequence([Scalar::from(100_i64)])]),
        )
        .expect("the row materializes");
        let reader = batch_reader(batch.schema(), [batch]);

        let value = ArrowValue::from_reader(reader)
            .expect("the reader names its root")
            .cast(&target, ArrowCastOptions::new())
            .expect("the plan compiles from the reader schema");

        assert!(value.is_stream());
        assert_eq!(
            value
                .into_reader()
                .expect("a stream is already a reader")
                .map(|batch| batch.expect("a batch reads").num_rows())
                .sum::<usize>(),
            1
        );
    }
}
