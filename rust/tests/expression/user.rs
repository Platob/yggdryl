//! User-defined functions: registered outside the grammar, typed and called through their signature.

use std::sync::Arc;

use yggdryl::expression::{
    Filter, FunctionSignature, Selector, Term, UserFunction, UserRef, lookup_function,
    register_function, registered_functions, unregister_function,
};
use yggdryl::{DataType, Field, Result, Scalar, StructureType};

/// `rs.double(value)`: twice an integer.
struct Double(FunctionSignature);

impl UserFunction for Double {
    fn signature(&self) -> &FunctionSignature {
        &self.0
    }

    fn call(&self, arguments: &[Scalar]) -> Result<Scalar> {
        let value = arguments[0].as_i64().unwrap_or_default();
        Ok(Scalar::from(value * 2))
    }
}

/// `rs.add(value, amount = 1)`: an integer plus a defaulted one.
struct Add(FunctionSignature);

impl UserFunction for Add {
    fn signature(&self) -> &FunctionSignature {
        &self.0
    }

    fn call(&self, arguments: &[Scalar]) -> Result<Scalar> {
        let value = arguments[0].as_i64().unwrap_or_default();
        let amount = arguments[1].as_i64().unwrap_or_default();
        Ok(Scalar::from(value + amount))
    }
}

fn registered() {
    let double = FunctionSignature::new(
        UserRef::new("rs", "double").unwrap(),
        [DataType::Int64.required_field("value")],
        DataType::Int64.nullable_field("returns"),
    )
    .unwrap();
    register_function(Arc::new(Double(double))).unwrap();
    let amount = FunctionSignature::with_default(
        DataType::Int64.required_field("amount"),
        &Scalar::from(1_i64),
    )
    .unwrap();
    let add = FunctionSignature::new(
        UserRef::new("rs", "add").unwrap(),
        [DataType::Int64.required_field("value"), amount],
        DataType::Int64.nullable_field("returns"),
    )
    .unwrap();
    register_function(Arc::new(Add(add))).unwrap();
}

fn rows() -> Field {
    StructureType::from_fields([
        DataType::Int64.nullable_field("size"),
        DataType::utf8().nullable_field("ccy"),
    ])
    .map(DataType::from)
    .unwrap()
    .required_field("rows")
}

#[test]
fn a_qualified_name_parses_prints_and_types_through_its_registration() {
    registered();
    let term: Term = "RS.Double(size)".parse().unwrap();
    assert_eq!(term.to_string(), "rs.double(size)");
    let typed = term.field(&rows()).unwrap();
    assert_eq!(typed.dtype(), &DataType::Int64);
    // A nullable argument meeting a `not null` parameter answers null.
    assert!(typed.is_nullable());
    assert_eq!(term.field(&rows()).unwrap().name(), "rs.double(size)");
}

#[test]
fn an_unregistered_function_is_refused_by_name_where_it_is_typed() {
    let term: Term = "nobody.knows(size)".parse().unwrap();
    let error = term.field(&rows()).unwrap_err().to_string();
    assert!(error.contains("nobody.knows"), "{error}");
    assert!(error.contains("register"), "{error}");
    let bad = UserRef::new("9lives", "f").unwrap_err().to_string();
    assert!(bad.contains("identifier"), "{bad}");
}

#[test]
fn the_scalar_and_vectorized_tiers_agree_and_defaults_fill_in() {
    registered();
    let selector: Selector =
        "rs.double(size) as doubled, rs.add(size) as next, rs.add(size, 10) as later"
            .parse()
            .unwrap();
    let root = rows();
    let bound = selector.bind(&root).unwrap();
    let row = bound
        .apply_scalar(&Scalar::from_sequence([
            Scalar::from(2_i64),
            Scalar::from("EUR"),
        ]))
        .unwrap();
    assert_eq!(
        row,
        Scalar::from_sequence([
            Scalar::from(4_i64),
            Scalar::from(3_i64),
            Scalar::from(12_i64)
        ])
    );
    let batch = arrow_array::RecordBatch::try_from_iter([
        (
            "size",
            Arc::new(arrow_array::Int64Array::from(vec![Some(1), None, Some(3)]))
                as arrow_array::ArrayRef,
        ),
        (
            "ccy",
            Arc::new(arrow_array::StringArray::from(vec!["a", "b", "c"])) as arrow_array::ArrayRef,
        ),
    ])
    .unwrap();
    let projected = selector.apply_arrow_batch(&batch).unwrap();
    let doubled = projected
        .column(0)
        .as_any()
        .downcast_ref::<arrow_array::Int64Array>()
        .unwrap();
    assert_eq!(doubled.iter().collect::<Vec<_>>(), [Some(2), None, Some(6)]);
    let filter: Filter = "rs.double(size) > 2".parse().unwrap();
    assert_eq!(filter.apply_arrow_batch(&batch).unwrap().num_rows(), 1);
}

#[test]
fn a_signature_is_a_struct_field_both_ways() {
    registered();
    let signature = lookup_function(&UserRef::parse("rs.add").unwrap())
        .unwrap()
        .signature()
        .clone();
    let field = signature.as_field().unwrap();
    assert_eq!(field.name(), "rs.add");
    assert_eq!(field.fields().len(), 2);
    assert_eq!(field.get_metadata("FUNCTION:returns"), Some("int64 null"));
    assert_eq!(
        field.fields()[1].get_metadata("FUNCTION:default"),
        Some("1")
    );
    assert_eq!(FunctionSignature::from_field(&field).unwrap(), signature);
    assert_eq!(signature.arity(), (1, 2));
    // A default has to trail.
    let leading = FunctionSignature::new(
        UserRef::new("rs", "wrong").unwrap(),
        [
            FunctionSignature::with_default(
                DataType::Int64.required_field("a"),
                &Scalar::from(1_i64),
            )
            .unwrap(),
            DataType::Int64.required_field("b"),
        ],
        DataType::Int64.nullable_field("returns"),
    );
    assert!(leading.unwrap_err().to_string().contains("default"));
}

#[test]
fn a_call_over_columns_is_stored_as_the_function_and_its_sources() {
    registered();
    let selector: Selector = "ccy, rs.double(size) as doubled".parse().unwrap();
    let stored = selector.into_field(&rows()).unwrap();
    let doubled = &stored.fields()[1];
    assert_eq!(
        doubled.get_metadata("TRANSFORM:function"),
        Some("rs.double")
    );
    assert_eq!(
        doubled.get_metadata("TRANSFORM:sources"),
        Some(r#"["size"]"#)
    );
    assert_eq!(doubled.get_metadata("TRANSFORM:expression"), None);
    assert_eq!(
        doubled.as_transform().term().unwrap().unwrap().to_string(),
        "rs.double(size)"
    );
    assert_eq!(
        Selector::from_field(&stored).to_string(),
        "ccy utf8 null, rs.double(size) as doubled int64 null"
    );
    // A grammar function over one column is stored the same way, and a
    // computed argument keeps the expression spelling.
    let mut year = DataType::Int32.nullable_field("year");
    year.as_transform_mut()
        .set_term(&"year(event)".parse().unwrap())
        .unwrap();
    assert_eq!(year.get_metadata("TRANSFORM:function"), Some("year"));
    let mut twice = DataType::Int64.nullable_field("twice");
    twice
        .as_transform_mut()
        .set_term(&"size * 2".parse().unwrap())
        .unwrap();
    assert_eq!(twice.get_metadata("TRANSFORM:expression"), Some("size * 2"));
    assert_eq!(twice.get_metadata("TRANSFORM:function"), None);
    assert!(
        unregister_function(&UserRef::parse("rs.double").unwrap())
            || registered_functions().is_empty()
    );
    registered();
}
