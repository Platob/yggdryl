//! Integration tests for [`DefaultScalar`] — the defaults flowing from the data
//! types into this layer's scalars.

use yggdryl_scalar::yggdryl_dtype::{self as dtype, DataType};
use yggdryl_scalar::{DefaultScalar, Int64, Map, Optional, RawScalar, Serie, UInt8};

type Int64ListScalar = Serie<dtype::Int64, Int64>;
type RankMap = Map<dtype::UInt8, dtype::Int64, UInt8, Int64>;

#[test]
fn defaults_flow_through_the_typed_layer() {
    // Integers default to zero, held in a value scalar.
    assert_eq!(dtype::Int64.default_value(), 0);
    assert_eq!(dtype::Int64.default_scalar(), Int64::new(0));

    // The optional's scalar models nullness: its default is the null variant.
    let optional = dtype::Optional::new(dtype::Int64);
    assert_eq!(optional.default_value(), 0);
    assert_eq!(
        optional.default_scalar(),
        Optional::<dtype::Int64, Int64>::null()
    );

    // Sequences default to empty, not null.
    assert_eq!(
        dtype::List::new(dtype::Int64).default_scalar(),
        Int64ListScalar::new(Vec::new())
    );
    assert_eq!(
        dtype::Map::new(dtype::UInt8, dtype::Int64).default_scalar(),
        RankMap::default()
    );

    // Binary defaults to the empty byte sequence.
    assert_eq!(
        dtype::Binary.default_scalar(),
        yggdryl_scalar::Binary::new(Vec::new())
    );
}

#[test]
fn defaults_reach_generic_code() {
    // Generic code bounds on DefaultScalar to build a type's default scalar.
    fn default_of<T, D: DefaultScalar<T>>(data_type: &D) -> D::Scalar {
        data_type.default_scalar()
    }
    assert_eq!(default_of(&dtype::Int64), Int64::new(0));
    assert!(!default_of(&dtype::List::new(dtype::Int64)).is_null());
    assert!(default_of(&dtype::Optional::new(dtype::Int64)).is_null());
}
