//! The [`RawField`] base trait: a named, nullable column of a
//! [`RawDataType`](yggdryl_dtype::RawDataType).

use yggdryl_dtype::{DataError, RawDataType};

/// A named, nullable column of a data type — the base trait mirroring an Apache
/// Arrow `Field`.
///
/// It pairs a [`name`](RawField::name) with a [`data_type`](RawField::data_type) of
/// type `D` and a [`is_nullable`](RawField::is_nullable) flag, so a schema is a
/// sequence of fields, and converts to and from the [`arrow_schema::Field`] it
/// mirrors ([`to_arrow`](RawField::to_arrow) / [`from_arrow`](RawField::from_arrow)).
/// It is parameterised by the data type `D` (rather than boxing it) so the concrete
/// type is preserved for zero-cost, monomorphised access, and carries
/// `Debug + Send + Sync` so a schema is printable and shareable across threads and
/// FFI.
///
/// ```
/// use yggdryl_field::yggdryl_dtype::{DataError, Int32, RawDataType};
/// use yggdryl_field::{arrow_schema, RawField};
///
/// #[derive(Debug)]
/// struct Column {
///     name: String,
///     data_type: Int32,
///     nullable: bool,
/// }
///
/// impl RawField<Int32> for Column {
///     fn name(&self) -> &str {
///         &self.name
///     }
///     fn data_type(&self) -> &Int32 {
///         &self.data_type
///     }
///     fn is_nullable(&self) -> bool {
///         self.nullable
///     }
///     fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError> {
///         // An extension type is a different logical type riding on metadata.
///         if let Some(extension) = field.metadata().get("ARROW:extension:name") {
///             return Err(DataError::IncompatibleArrowType {
///                 expected: "Int32".to_string(),
///                 got: format!("the extension type \"{extension}\""),
///             });
///         }
///         Ok(Column {
///             name: field.name().to_string(),
///             data_type: Int32::from_arrow(field.data_type())?,
///             nullable: field.is_nullable(),
///         })
///     }
/// }
///
/// let id = Column { name: "id".to_string(), data_type: Int32, nullable: false };
/// assert_eq!(id.name(), "id");
/// assert_eq!(id.data_type().name(), "int32");
/// assert!(!id.is_nullable());
///
/// // `to_arrow` (defaulted from the three accessors) round-trips through Arrow.
/// let arrow = id.to_arrow();
/// assert_eq!(arrow, arrow_schema::Field::new("id", arrow_schema::DataType::Int32, false));
/// assert_eq!(Column::from_arrow(&arrow).unwrap().name(), "id");
/// ```
pub trait RawField<D: RawDataType>: std::fmt::Debug + Send + Sync {
    /// The field's name.
    fn name(&self) -> &str;

    /// The field's data type.
    fn data_type(&self) -> &D;

    /// Whether values in this field may be null.
    fn is_nullable(&self) -> bool;

    /// The [`arrow_schema::Field`] this field mirrors: same name, data type and
    /// nullability. Defaults to building it from those three accessors.
    ///
    /// The model carries exactly those three properties — a field never holds Arrow
    /// metadata, so the produced `Field` has none.
    fn to_arrow(&self) -> arrow_schema::Field {
        arrow_schema::Field::new(self.name(), self.data_type().to_arrow(), self.is_nullable())
    }

    /// Build this field from the [`arrow_schema::Field`] it mirrors — the exact
    /// inverse of [`to_arrow`](RawField::to_arrow) on the metadata-free fields that
    /// method produces. A field of a different Arrow data type errors with
    /// [`DataError::IncompatibleArrowType`].
    ///
    /// Arrow metadata is handled in two tiers: a field carrying an
    /// `ARROW:extension:name` entry is a *different* logical type and is refused
    /// with [`DataError::IncompatibleArrowType`]; any other metadata is not modeled
    /// (see [`to_arrow`](RawField::to_arrow)) and is deliberately dropped — logged
    /// as a `warn` when the `log` cargo feature is on.
    fn from_arrow(field: &arrow_schema::Field) -> Result<Self, DataError>
    where
        Self: Sized;
}

/// The shared `from_arrow` metadata policy every concrete field applies before
/// rebuilding: refuse an extension-typed field (`ARROW:extension:name` is a
/// different logical type) with [`DataError::IncompatibleArrowType`] naming
/// `expected`, and log dropped non-extension metadata as a `warn`.
pub(crate) fn validate_field_metadata(
    field: &arrow_schema::Field,
    expected: &str,
) -> Result<(), DataError> {
    if let Some(extension) = field.metadata().get("ARROW:extension:name") {
        return Err(DataError::IncompatibleArrowType {
            expected: expected.to_string(),
            got: format!(
                "the extension type \"{extension}\" (stored as {})",
                field.data_type()
            ),
        });
    }
    if !field.metadata().is_empty() {
        crate::log_event!(
            warn,
            "from_arrow: dropping {} metadata entr(y/ies) from field \"{}\"",
            field.metadata().len(),
            field.name()
        );
    }
    Ok(())
}
