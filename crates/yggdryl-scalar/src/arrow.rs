//! Apache Arrow interoperability for the dynamic / nested schema.
//!
//! The primitive types round-trip in the `yggdryl-schema` crate; here the dynamic
//! [`AnyType`] / [`AnyField`] and the recursive [`StructType`] / [`StructField`] carry
//! the full [`to_arrow`](StructField::to_arrow) / [`from_arrow`](StructField::from_arrow)
//! round-trip (names, nullability, nesting and metadata), built on the schema crate's
//! [`ArrowSchema`] / [`ArrowArray`] nodes and its public helpers. Because an Arrow
//! schema *is* a [`StructField`], nesting is fully recursive.
//!
//! ```
//! use yggdryl_scalar::{AnyField, AnyType, StructField};
//! use yggdryl_schema::DataTypeId;
//!
//! let schema = StructField::new(
//!     "record",
//!     vec![
//!         AnyField::new("id", AnyType::primitive(DataTypeId::Int64)),
//!         AnyField::new("big", AnyType::primitive(DataTypeId::Int128)),
//!     ],
//! );
//! let arrow = schema.to_arrow();
//! assert_eq!(arrow.format(), "+s");
//! assert_eq!(StructField::from_arrow(&arrow).unwrap(), schema); // lossless
//! ```

use yggdryl_schema::{ArrowArray, ArrowError, ArrowSchema, DataTypeId, Field, Metadata};

use crate::{AnyField, AnyType, StructField, StructType};

/// Checks that an Arrow format string denotes a struct.
fn expect_struct(format: &str) -> Result<(), ArrowError> {
    if format == DataTypeId::Struct.arrow_format() {
        Ok(())
    } else {
        Err(ArrowError::NotAStruct(format.to_owned()))
    }
}

impl AnyType {
    /// The type encoded as an Arrow node (with no name).
    pub fn to_arrow(&self) -> ArrowSchema {
        match self {
            AnyType::Primitive(id) => ArrowSchema::primitive(*id),
            AnyType::Struct(ty) => ty.to_arrow(),
        }
    }

    /// The type built from an Arrow node, or an [`ArrowError`] if its format is one
    /// this schema layer does not model.
    pub fn from_arrow(schema: &ArrowSchema) -> Result<Self, ArrowError> {
        if schema.format() == DataTypeId::Struct.arrow_format() {
            StructType::from_arrow(schema).map(AnyType::Struct)
        } else {
            schema.primitive_id().map(AnyType::Primitive)
        }
    }

    /// The type built from a `(schema, array)` pair. Identical to [`from_arrow`](AnyType::from_arrow)
    /// for a primitive; for a struct it threads the child arrays through so each child
    /// field takes its nullability from the data.
    pub fn from_arrow_array(schema: &ArrowSchema, array: &ArrowArray) -> Result<Self, ArrowError> {
        if schema.format() == DataTypeId::Struct.arrow_format() {
            StructType::from_arrow_array(schema, array).map(AnyType::Struct)
        } else {
            schema.primitive_id().map(AnyType::Primitive)
        }
    }
}

impl StructType {
    /// The struct type encoded as an Arrow `"+s"` node whose children are its fields.
    pub fn to_arrow(&self) -> ArrowSchema {
        ArrowSchema::from_parts(
            DataTypeId::Struct.arrow_format().to_owned(),
            String::new(),
            false,
            Metadata::new(),
            self.fields().iter().map(AnyField::to_arrow).collect(),
        )
    }

    /// The struct type built from an Arrow `"+s"` node, recursing into its children.
    pub fn from_arrow(schema: &ArrowSchema) -> Result<Self, ArrowError> {
        expect_struct(schema.format())?;
        let fields = schema
            .children()
            .iter()
            .map(AnyField::from_arrow)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(StructType::new(fields))
    }

    /// The struct type built from a `(schema, array)` pair, pairing each child schema
    /// with its child array so every child field's nullability comes from the data.
    /// Errors if the schema and array disagree on the number of children.
    pub fn from_arrow_array(schema: &ArrowSchema, array: &ArrowArray) -> Result<Self, ArrowError> {
        expect_struct(schema.format())?;
        if schema.children().len() != array.children().len() {
            return Err(ArrowError::ChildCountMismatch {
                schema: schema.children().len(),
                array: array.children().len(),
            });
        }
        let fields = schema
            .children()
            .iter()
            .zip(array.children().iter())
            .map(|(s, a)| AnyField::from_arrow_array(s, a))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(StructType::new(fields))
    }
}

impl AnyField {
    /// The field encoded as an Arrow node (its type, stamped with name/nullability
    /// and metadata).
    pub fn to_arrow(&self) -> ArrowSchema {
        ArrowSchema::field(
            self.any_type().to_arrow(),
            self.name(),
            self.nullable(),
            self.metadata(),
        )
    }

    /// The field built from an Arrow node, or an [`ArrowError`] on an unmodelled type.
    pub fn from_arrow(schema: &ArrowSchema) -> Result<Self, ArrowError> {
        let dtype = AnyType::from_arrow(schema)?;
        Ok(AnyField::from_parts(
            schema.name().to_owned(),
            dtype,
            schema.nullable(),
            schema.field_metadata(),
        ))
    }

    /// The field built from a `(schema, array)` pair. Unlike [`from_arrow`](AnyField::from_arrow),
    /// which trusts the schema's flag, this takes the field's nullability from the
    /// array's `null_count` — the nulls the data actually holds.
    pub fn from_arrow_array(schema: &ArrowSchema, array: &ArrowArray) -> Result<Self, ArrowError> {
        let dtype = AnyType::from_arrow_array(schema, array)?;
        Ok(AnyField::from_parts(
            schema.name().to_owned(),
            dtype,
            array.nullable(),
            schema.field_metadata(),
        ))
    }
}

impl StructField {
    /// The schema encoded as an Arrow `"+s"` node — the canonical Arrow schema form.
    pub fn to_arrow(&self) -> ArrowSchema {
        ArrowSchema::field(
            self.dtype().to_arrow(),
            self.name(),
            self.nullable(),
            self.metadata(),
        )
    }

    /// The schema built from an Arrow `"+s"` node, or an [`ArrowError`] if the node is
    /// not a struct or holds an unmodelled child type.
    pub fn from_arrow(schema: &ArrowSchema) -> Result<Self, ArrowError> {
        let dtype = StructType::from_arrow(schema)?;
        Ok(StructField::from_parts(
            schema.name().to_owned(),
            dtype,
            schema.nullable(),
            schema.field_metadata(),
        ))
    }

    /// The schema built from a `(schema, array)` pair, taking this struct's — and,
    /// recursively, every child field's — nullability from the arrays' `null_count`.
    pub fn from_arrow_array(schema: &ArrowSchema, array: &ArrowArray) -> Result<Self, ArrowError> {
        let dtype = StructType::from_arrow_array(schema, array)?;
        Ok(StructField::from_parts(
            schema.name().to_owned(),
            dtype,
            array.nullable(),
            schema.field_metadata(),
        ))
    }
}

/// Converts between an [`ArrowSchema`] node and a [`StructField`] — the canonical
/// schema type — from the node's side (the counterpart of [`StructField::to_arrow`] /
/// [`StructField::from_arrow`]).
pub trait ArrowSchemaExt {
    /// This Arrow schema built from a [`StructField`].
    fn from_struct_field(field: &StructField) -> Self;

    /// This Arrow schema converted to a [`StructField`], or an [`ArrowError`] if it is
    /// not a struct (`"+s"`) node or holds an unmodelled child type.
    fn to_struct_field(&self) -> Result<StructField, ArrowError>;
}

impl ArrowSchemaExt for ArrowSchema {
    fn from_struct_field(field: &StructField) -> Self {
        field.to_arrow()
    }

    fn to_struct_field(&self) -> Result<StructField, ArrowError> {
        StructField::from_arrow(self)
    }
}
