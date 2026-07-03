//! The [`Union`] data type.

use crate::{DataError, RawDataType, RawNested};
use arrow_schema::{UnionFields, UnionMode};

/// The Apache Arrow `union` data type: a value is exactly one of several child
/// types, discriminated by a type id.
///
/// It carries its [`UnionFields`] — the `(type id, child field)` pairs — and its
/// [`UnionMode`] (`Sparse` or `Dense`), exactly as Arrow models them, so
/// [`to_arrow`](RawDataType::to_arrow) / [`from_arrow`](RawDataType::from_arrow)
/// round-trip losslessly. It is a [`RawNested`] type: its children are fields and it
/// has no fixed width of its own.
///
/// [`Union::optional`] builds the two-variant union between [`Null`](crate::Null)
/// and a value type — the shape [`Optional`](crate::Optional) is built on.
///
/// ```
/// use yggdryl_dtype::{arrow_schema, Int64, RawDataType, RawNested, Union};
///
/// // A union of null and int64 (the "optional int64" shape).
/// let union = Union::optional(&Int64);
/// assert_eq!(union.name(), "union");
/// assert_eq!(union.arrow_format(), "+us:0,1"); // sparse, type ids 0 and 1
/// assert_eq!(union.byte_width(), None);
/// assert_eq!(union.child_count(), 2);
///
/// // to_arrow / from_arrow are lossless.
/// let arrow = union.to_arrow();
/// assert!(matches!(arrow, arrow_schema::DataType::Union(..)));
/// assert_eq!(Union::from_arrow(&arrow).unwrap(), union);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Union {
    fields: UnionFields,
    mode: UnionMode,
}

impl Union {
    /// This type's [`DataTypeId`](crate::DataTypeId).
    pub const ID: crate::DataTypeId = crate::DataTypeId::Union;

    /// The type id of the null variant in a [`Union::optional`] union.
    pub const NULL_TYPE_ID: i8 = 0;

    /// The type id of the value variant in a [`Union::optional`] union.
    pub const VALUE_TYPE_ID: i8 = 1;

    /// A union of the given `(type id, child field)` pairs in `mode`.
    pub fn new(fields: UnionFields, mode: UnionMode) -> Self {
        Self { fields, mode }
    }

    /// The sparse two-variant union between null and `value_type`: type id
    /// [`NULL_TYPE_ID`](Union::NULL_TYPE_ID) is a [`Null`](crate::Null) child named
    /// `"null"`, and [`VALUE_TYPE_ID`](Union::VALUE_TYPE_ID) is a `value_type`
    /// child named after the type.
    pub fn optional(value_type: &dyn RawDataType) -> Self {
        // The null child is identical for every optional union: built once,
        // shared by reference count.
        static NULL_FIELD: std::sync::OnceLock<arrow_schema::FieldRef> = std::sync::OnceLock::new();
        let null_field = NULL_FIELD
            .get_or_init(|| {
                std::sync::Arc::new(arrow_schema::Field::new(
                    "null",
                    arrow_schema::DataType::Null,
                    true,
                ))
            })
            .clone();
        let value_field = std::sync::Arc::new(arrow_schema::Field::new(
            value_type.name(),
            value_type.to_arrow(),
            false,
        ));
        let fields = UnionFields::try_new(
            [Self::NULL_TYPE_ID, Self::VALUE_TYPE_ID],
            [null_field, value_field],
        )
        .expect("two distinct type ids and two fields form valid union fields");
        Self::new(fields, UnionMode::Sparse)
    }
}

impl super::RawUnion for Union {
    fn fields(&self) -> &UnionFields {
        &self.fields
    }

    fn mode(&self) -> UnionMode {
        self.mode
    }
}

impl RawDataType for Union {
    fn name(&self) -> &str {
        "union"
    }

    fn arrow_format(&self) -> String {
        // C Data Interface: "+us:<type ids>" (sparse) or "+ud:<type ids>" (dense).
        let ids: Vec<String> = self.fields.iter().map(|(id, _)| id.to_string()).collect();
        let mode = match self.mode {
            UnionMode::Sparse => 's',
            UnionMode::Dense => 'd',
        };
        format!("+u{mode}:{}", ids.join(","))
    }

    fn byte_width(&self) -> Option<usize> {
        None
    }

    fn to_arrow(&self) -> arrow_schema::DataType {
        arrow_schema::DataType::Union(self.fields.clone(), self.mode)
    }

    fn from_arrow(data_type: &arrow_schema::DataType) -> Result<Self, DataError> {
        match data_type {
            arrow_schema::DataType::Union(fields, mode) => Ok(Self::new(fields.clone(), *mode)),
            other => Err(DataError::IncompatibleArrowType {
                expected: "Union".to_string(),
                got: other.to_string(),
            }),
        }
    }
}

impl RawNested for Union {
    fn child_count(&self) -> usize {
        self.fields.len()
    }
}
