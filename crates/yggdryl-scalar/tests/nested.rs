//! Integration tests for the nested access surface: [`RecordScalar`], the
//! [`NestedSerie`] child accessors, and the base [`Scalar`]'s `as_serie` /
//! `as_map` / `as_struct` accessors.

use std::sync::Arc;

use yggdryl_scalar::yggdryl_dtype::{self as dtype, arrow_schema, DataError, DataType};
use yggdryl_scalar::{
    arrow_array, AnyScalar, AnySerie, Int64Scalar, Int64Serie, MapScalar, NestedSerie,
    RecordScalar, Scalar, StructScalar, StructSerie, TypedMapScalar, TypedOptionalScalar,
    TypedSerie, TypedStructSerie, UInt8Scalar,
};

fn point_type() -> dtype::StructType {
    dtype::StructType::new(arrow_schema::Fields::from(vec![
        arrow_schema::Field::new("x", arrow_schema::DataType::Int64, false),
        arrow_schema::Field::new("y", arrow_schema::DataType::Int64, false),
    ]))
}

fn point(x: i64, y: i64) -> RecordScalar {
    RecordScalar::new(
        point_type(),
        vec![
            AnyScalar::from(Int64Scalar::new(x)),
            AnyScalar::from(Int64Scalar::new(y)),
        ],
    )
    .unwrap()
}

fn point_record() -> RecordScalar {
    point(1, 2)
}

#[test]
fn record_gives_generic_child_scalar_access() {
    let row = point_record();
    assert_eq!(row.data_type().name(), "struct");
    assert!(!row.is_null());

    // By position and by field name; each field is its own atomic scalar.
    assert_eq!(
        row.scalar_at(0).unwrap(),
        AnyScalar::from(Int64Scalar::new(1))
    );
    assert_eq!(
        row.scalar_by("y").unwrap(),
        AnyScalar::from(Int64Scalar::new(2))
    );
    assert!(row.scalar_at(2).is_none());
    assert!(row.scalar_by("z").is_none());

    // The Arrow round trip preserves the row; a null record round-trips too.
    assert_eq!(
        RecordScalar::from_arrow(row.to_arrow_scalar().as_ref()).unwrap(),
        row
    );
    let missing = RecordScalar::null(point_type());
    assert!(missing.is_null());
    assert!(missing.scalar_at(0).is_none());
    assert_eq!(
        RecordScalar::from_arrow(missing.to_arrow_scalar().as_ref()).unwrap(),
        missing
    );
}

#[test]
fn record_construction_is_validated() {
    // Arity and per-scalar type both validated.
    assert!(matches!(
        RecordScalar::new(
            point_type(),
            vec![AnyScalar::from(Int64Scalar::new(1))] // one scalar, two fields
        ),
        Err(DataError::IncompatibleArrowType { .. })
    ));
    assert!(matches!(
        RecordScalar::new(
            point_type(),
            vec![
                AnyScalar::from(UInt8Scalar::new(1)), // uint8 where int64 declared
                AnyScalar::from(Int64Scalar::new(2)),
            ],
        ),
        Err(DataError::IncompatibleArrowType { .. })
    ));
}

#[test]
fn serie_children_are_the_item_serie() {
    let numbers = TypedSerie::new(vec![Int64Scalar::new(1), Int64Scalar::new(2)]);
    assert_eq!(numbers.child_serie_count(), 1);
    assert_eq!(numbers.child_serie_name_at(0).as_deref(), Some("item"));
    assert_eq!(numbers.child_serie_at(0).unwrap().len(), 2);
    assert_eq!(numbers.child_serie_by("item").unwrap().len(), 2);
    assert!(numbers.child_serie_by("entries").is_none());
    assert!(numbers.child_serie_at(1).is_none());

    // The item serie is decomposed — built from int64 scalars it lands as Int64.
    assert!(matches!(
        numbers.child_serie_at(0).unwrap(),
        AnySerie::Int64(_)
    ));

    // A null serie has no child series.
    let missing: TypedSerie<dtype::Int64Type, Int64Scalar> = TypedSerie::null();
    assert!(missing.child_serie_at(0).is_none());

    // The dynamic and concrete series answer the same children.
    assert_eq!(numbers.erase().child_serie_at(0).unwrap().len(), 2);
    let concrete = Int64Serie::from(vec![1, 2]);
    assert_eq!(
        concrete
            .as_serie()
            .unwrap()
            .child_serie_at(0)
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn map_children_are_entries_with_key_value_projections() {
    let ranks = TypedMapScalar::new(vec![
        (UInt8Scalar::new(7), Int64Scalar::new(42)),
        (UInt8Scalar::new(8), Int64Scalar::new(43)),
    ])
    .unwrap();

    // The typed and dynamic maps agree: one "entries" child of two rows.
    for children in [&ranks as &dyn NestedSerie, &ranks.erase()] {
        assert_eq!(children.child_serie_count(), 1);
        assert_eq!(children.child_serie_name_at(0).as_deref(), Some("entries"));
        assert_eq!(children.child_serie_at(0).unwrap().len(), 2);
        // The key / value projections decompose into their own series.
        let keys = children.child_serie_by("key").unwrap();
        assert!(matches!(keys, AnySerie::UInt8(_)));
        let values = children.child_serie_by("value").unwrap();
        assert_eq!(values.len(), 2);
        assert!(children.child_serie_by("missing").is_none());
    }
}

#[test]
fn struct_children_are_the_columns_by_name() {
    let row = StructScalar::new(
        point_type(),
        vec![
            Arc::new(arrow_array::Int64Array::from_iter_values([1])),
            Arc::new(arrow_array::Int64Array::from_iter_values([2])),
        ],
    )
    .unwrap();
    assert_eq!(row.child_serie_count(), 2);
    assert_eq!(row.child_serie_name_at(1).as_deref(), Some("y"));
    assert!(matches!(
        row.child_serie_by("x").unwrap(),
        AnySerie::Int64(_)
    ));
    assert!(row.child_serie_by("z").is_none());

    // The record view shares the same children.
    let record = row.as_struct().unwrap();
    assert_eq!(record, point_record());
    assert_eq!(record.child_serie_by("x"), row.child_serie_by("x"));
}

#[test]
fn struct_serie_holds_rows_and_field_columns() {
    let points = TypedStructSerie::new(point_type(), vec![point(1, 2), point(3, 4)]);
    assert_eq!(points.len(), 2);
    assert!(!points.is_null());
    assert_eq!(points.data_type().name(), "list");

    // Rows come back as records, by position.
    assert_eq!(points.get_scalar_at(0), Some(point(1, 2)));
    assert_eq!(points.get_scalar_at(1), Some(point(3, 4)));
    assert!(points.get_scalar_at(2).is_none());

    // The children are the struct's field columns, not one "item" child.
    assert_eq!(points.child_serie_count(), 2);
    assert_eq!(points.child_serie_name_at(0).as_deref(), Some("x"));
    let xs = points.child_serie_by("x").unwrap();
    assert_eq!(xs.len(), 2); // "x" across both rows
    assert!(matches!(xs, AnySerie::Int64(_)));
    assert!(points.child_serie_by("z").is_none());

    // The Arrow round trip shares the buffers.
    assert_eq!(
        TypedStructSerie::from_arrow(points.to_arrow_scalar().as_ref()).unwrap(),
        points
    );

    // Erase to the dynamic StructSerie — rows read back as records, fields as columns.
    let dynamic = points.erase();
    assert_eq!(dynamic.len(), 2);
    assert_eq!(dynamic.get_row(1), Some(point(3, 4)));
    assert_eq!(dynamic.child_serie_by("y").unwrap().len(), 2);
    assert_eq!(
        StructSerie::from_arrow(dynamic.to_arrow_scalar().as_ref()).unwrap(),
        dynamic
    );

    // as_serie hands back the generic dynamic list of the same rows.
    assert_eq!(points.as_serie().unwrap().len(), 2);
}

#[test]
fn struct_serie_row_type_is_generic_over_scalar() {
    // The row scalar type is a parameter: a serie read back as the column-oriented
    // StructScalar rather than the RecordScalar row atom.
    let columns: TypedStructSerie<StructScalar> = TypedStructSerie::new(
        point_type(),
        vec![StructScalar::new(
            point_type(),
            vec![
                Arc::new(arrow_array::Int64Array::from_iter_values([1])),
                Arc::new(arrow_array::Int64Array::from_iter_values([2])),
            ],
        )
        .unwrap()],
    );
    assert_eq!(columns.len(), 1);
    assert_eq!(
        columns.get_scalar_at(0).unwrap().as_struct().unwrap(),
        point(1, 2)
    );
}

#[test]
fn struct_serie_null_and_empty_are_distinct() {
    let missing: TypedStructSerie<RecordScalar> = TypedStructSerie::null(point_type());
    assert!(missing.is_null());
    assert_eq!(missing.len(), 0);
    assert!(missing.get_scalar_at(0).is_none());
    // Even null, the field columns are named from the item struct.
    assert_eq!(missing.child_serie_count(), 2);
    assert!(missing.child_serie_at(0).is_none());
    assert_eq!(
        TypedStructSerie::<RecordScalar>::from_arrow(missing.to_arrow_scalar().as_ref()).unwrap(),
        missing
    );

    let empty = TypedStructSerie::<RecordScalar>::new(point_type(), vec![]);
    assert!(!empty.is_null()); // empty is not null
    assert!(empty.is_empty());
    assert_eq!(empty.len(), 0);
    assert_ne!(empty, missing);
}

#[test]
fn struct_serie_carries_a_null_row_among_present_ones() {
    // A row-level null (the whole struct absent) is distinct from a null serie: the
    // serie is present, only its middle row is null. This exercises the validity
    // bitmap through the concat assembly and the sliced `from_arrow` read-back.
    let rows = vec![point(1, 2), RecordScalar::null(point_type()), point(5, 6)];
    let serie = TypedStructSerie::new(point_type(), rows);
    assert_eq!(serie.len(), 3);
    assert!(!serie.is_null());

    // The present rows read back; the middle row reads back as a null record.
    assert_eq!(serie.get_scalar_at(0), Some(point(1, 2)));
    assert!(serie.get_scalar_at(1).expect("the row exists").is_null());
    assert_eq!(serie.get_scalar_at(2), Some(point(5, 6)));

    // The field column still spans every row (the null row included).
    assert_eq!(serie.child_serie_by("x").unwrap().len(), 3);

    // The Arrow round trip and the erased view both preserve the null row.
    assert_eq!(
        TypedStructSerie::<RecordScalar>::from_arrow(serie.to_arrow_scalar().as_ref()).unwrap(),
        serie
    );
    assert_eq!(serie.erase().get_row(1).expect("the row exists"), {
        RecordScalar::null(point_type())
    });
}

#[test]
fn as_nested_accessors_follow_the_as_contract() {
    // as_serie: every serie shape answers; the handles agree through Arrow.
    let typed = TypedSerie::new(vec![Int64Scalar::new(1), Int64Scalar::new(2)]);
    let concrete = Int64Serie::from(vec![1, 2]);
    assert_eq!(typed.as_serie().unwrap(), concrete.as_serie().unwrap());
    assert_eq!(concrete.as_serie().unwrap().len(), 2);

    // A null concrete serie stays a null dynamic serie.
    assert!(Int64Serie::null().as_serie().unwrap().is_null());

    // as_map: typed erases to the dynamic map.
    let ranks = TypedMapScalar::new(vec![(UInt8Scalar::new(7), Int64Scalar::new(42))]).unwrap();
    assert_eq!(ranks.as_map().unwrap(), ranks.erase());
    assert_eq!(
        MapScalar::from_arrow(ranks.to_arrow_scalar().as_ref())
            .unwrap()
            .as_map()
            .unwrap(),
        ranks.erase()
    );

    // as_struct: struct and record agree.
    assert_eq!(point_record().as_struct().unwrap(), point_record());

    // The optional redirects to its inner scalar; a scalar without the shape errors.
    let optional = TypedOptionalScalar::new(Int64Serie::from(vec![1, 2]));
    assert_eq!(optional.as_serie().unwrap().len(), 2);
    assert!(matches!(
        Int64Scalar::new(1).as_serie(),
        Err(DataError::UnsupportedConversion { .. })
    ));
    assert!(matches!(
        Int64Scalar::new(1).as_map(),
        Err(DataError::UnsupportedConversion { .. })
    ));
    assert!(matches!(
        Int64Scalar::new(1).as_struct(),
        Err(DataError::UnsupportedConversion { .. })
    ));
}

#[test]
fn nested_cast_dtype_covers_identity_and_refusal() {
    // Identity cast: a serie cast to its own type is its scalar form.
    let numbers = Int64Serie::from(vec![1, 2]);
    let cast = numbers
        .cast_dtype(&dtype::TypedSerieType::new(dtype::Int64Type))
        .unwrap();
    assert_eq!(Int64Serie::from_arrow(cast.as_ref()).unwrap(), numbers);

    // A record cast to a numeric target is refused with the `as_*` contract.
    assert!(matches!(
        point_record().cast_dtype(&dtype::Int64Type),
        Err(DataError::UnsupportedConversion { .. })
    ));
    // A scalar cast to a nested target is refused as an unsupported cast.
    assert!(matches!(
        Int64Scalar::new(1).cast_dtype(&point_type()),
        Err(DataError::UnsupportedCast { .. })
    ));
    // A null record still casts to a null of any castable target.
    let cast = RecordScalar::null(point_type())
        .cast_dtype(&dtype::Int64Type)
        .unwrap();
    assert!(arrow_array::Array::is_null(cast.as_ref(), 0));
}
