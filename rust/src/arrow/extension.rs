//! arrow-rs's typed extension types for the datatypes Arrow cannot state
//! alone: [`ExtensionType`] for each parameter-free datatype marker that
//! rides a name - the codes, the enum leaves, the uuid, the variant, the
//! version, the timezone and the MIME and media types - and for the
//! [`StringType`] and [`BytesType`] views, whose document is the leaf.
//!
//! Every method redirects into the one recognizer a field is imported
//! through, so `field.try_extension_type::<CcyType>()` and
//! [`Field::from_arrow_field`](crate::Field::from_arrow_field) answer alike:
//! the name over the storage the datatype lays out is that datatype, a
//! dictionary of it included, and any other storage is refused.

use std::collections::HashMap;

use arrow_schema::extension::{
    EXTENSION_TYPE_METADATA_KEY, EXTENSION_TYPE_NAME_KEY, ExtensionType,
};
use arrow_schema::{ArrowError, DataType as ArrowDataType};

use crate::{BytesType, DataType, DataTypeId, StringType};

/// The datatype `name`, `document` and `storage` state, as a field of them
/// imports; `None` where they state none of this crate's.
fn recognized(
    name: &str,
    document: Option<&str>,
    storage: &ArrowDataType,
) -> Result<Option<DataType>, ArrowError> {
    let mut metadata = HashMap::with_capacity(2);
    metadata.insert(EXTENSION_TYPE_NAME_KEY.to_owned(), name.to_owned());
    if let Some(document) = document {
        metadata.insert(EXTENSION_TYPE_METADATA_KEY.to_owned(), document.to_owned());
    }
    crate::field::recognized_arrow_extension(&metadata, storage, name)
        .map(|recognized| recognized.map(crate::field::RecognizedExtension::into_dtype))
        .map_err(|error| ArrowError::InvalidArgumentError(error.to_string()))
}

/// The refusal of a storage `name` does not lay its datatype out in.
fn unsupported(name: &str, storage: &ArrowDataType) -> ArrowError {
    ArrowError::InvalidArgumentError(format!(
        "expected the storage {name} lays its datatype out in, got {storage}"
    ))
}

/// The refusal of a document an extension states none of.
fn unexpected_document(name: &str, document: &str) -> ArrowError {
    ArrowError::InvalidArgumentError(format!("expected no {name} document, got {document:?}"))
}

/// What a parameter-free marker's `deserialize_metadata` answers: no
/// document, or the refusal of the one stated.
pub fn marker_metadata(name: &str, metadata: Option<&str>) -> Result<(), ArrowError> {
    match metadata {
        None | Some("") => Ok(()),
        Some(document) => Err(unexpected_document(name, document)),
    }
}

/// What a parameter-free marker's `supports_data_type` answers: the storage
/// the recognizer reads as the datatype `id` names, under the name `name`.
pub fn marker_supports(
    name: &str,
    id: DataTypeId,
    data_type: &ArrowDataType,
) -> Result<(), ArrowError> {
    match recognized(name, None, data_type)? {
        Some(dtype) if dtype.id() == id => Ok(()),
        _ => Err(unsupported(name, data_type)),
    }
}

/// [`ExtensionType`] for parameter-free markers: the name the datatype's
/// identifier rides, no document, and the storage the recognizer reads as
/// that datatype. A core marker reads its name off its identifier; a
/// registered enum kind's marker is the claiming crate's, written beside
/// its type through `market_extension!` below.
macro_rules! marker_extension {
    (@one $marker:ident => $variant:ident) => {
        marker_extension!(
            @impl $marker,
            match crate::DataTypeId::$variant.core_arrow_extension_name() {
                Some(name) => name,
                None => panic!("a marker's datatype rides an extension name"),
            },
            crate::DataTypeId::$variant
        );
    };
    (@impl $marker:ident, $name:expr, $id:expr) => {
        impl ExtensionType for crate::$marker {
            const NAME: &'static str = $name;

            type Metadata = ();

            fn metadata(&self) -> &Self::Metadata {
                &()
            }

            fn serialize_metadata(&self) -> Option<String> {
                None
            }

            fn deserialize_metadata(metadata: Option<&str>) -> Result<Self::Metadata, ArrowError> {
                marker_metadata(Self::NAME, metadata)
            }

            fn supports_data_type(&self, data_type: &ArrowDataType) -> Result<(), ArrowError> {
                marker_supports(Self::NAME, $id, data_type)
            }

            fn try_new(data_type: &ArrowDataType, (): Self::Metadata) -> Result<Self, ArrowError> {
                Self.supports_data_type(data_type).map(|()| Self)
            }
        }
    };
    ($($marker:ident => $($leaf:ident)+),* $(,)?) => {$(
        marker_extension!(@one $marker => $($leaf)+);
    )*};
}

marker_extension! {
    UuidType => Uuid,
    VariantType => Variant,
    VersionType => Version,
    TimezoneType => Timezone,
    MimeTypeType => MimeType,
    MediaTypeType => MediaType,
    StateType => State,
    CountryType => Country,
    CcyType => Ccy,
    MicType => Mic,
    CfiType => Cfi,
    IsinType => Isin,
    CusipType => Cusip,
    SedolType => Sedol,
    FigiType => Figi,
    RicType => Ric,
    BbgType => Bbg,
    UnitType => Unit,
    ForexType => Forex,
    LeiType => Lei,
    BicType => Bic,
    ElfType => Elf,
    DtiType => Dti,
    FisnType => Fisn,
}

/// [`ExtensionType`] for a registered enum kind's marker, written by the
/// crate that claims the kind beside its type - a foreign trait is
/// implemented only in the type's crate - as
/// `market_extension!(SideType, Side)`: the name and the identifier are the
/// kind's own, the storage the one the recognizer reads as that kind.
#[macro_export]
#[doc(hidden)]
macro_rules! market_extension {
    ($marker:ident, $leaf:ident) => {
        impl $crate::implementer::ExtensionType for $marker {
            const NAME: &'static str = <$leaf>::EXTENSION_NAME;

            type Metadata = ();

            fn metadata(&self) -> &Self::Metadata {
                &()
            }

            fn serialize_metadata(&self) -> Option<String> {
                None
            }

            fn deserialize_metadata(
                metadata: Option<&str>,
            ) -> Result<Self::Metadata, $crate::implementer::ArrowError> {
                $crate::implementer::marker_metadata(Self::NAME, metadata)
            }

            fn supports_data_type(
                &self,
                data_type: &$crate::implementer::ArrowDataType,
            ) -> Result<(), $crate::implementer::ArrowError> {
                $crate::implementer::marker_supports(Self::NAME, <$leaf>::ID, data_type)
            }

            fn try_new(
                data_type: &$crate::implementer::ArrowDataType,
                (): Self::Metadata,
            ) -> Result<Self, $crate::implementer::ArrowError> {
                Self.supports_data_type(data_type).map(|()| Self)
            }
        }
    };
}

/// [`ExtensionType`] for a view whose leaf is its document: `yggdryl.string`
/// and `yggdryl.bytes`, the leaf written whole and read back over the
/// storage it lays out. A leaf Arrow states alone - `utf8`, `binary` - rides
/// no document, so it supports no storage.
macro_rules! leaf_extension {
    ($view:ident, $parameters:ident, $name:expr) => {
        impl ExtensionType for $view {
            const NAME: &'static str = $name;

            type Metadata = Self;

            fn metadata(&self) -> &Self::Metadata {
                self
            }

            fn serialize_metadata(&self) -> Option<String> {
                Some(self.extension_json())
            }

            fn deserialize_metadata(metadata: Option<&str>) -> Result<Self::Metadata, ArrowError> {
                let document = metadata.ok_or_else(|| {
                    ArrowError::InvalidArgumentError(format!(
                        "expected the {} document naming the leaf, got none",
                        Self::NAME
                    ))
                })?;
                Self::from_extension_json(document)
                    .map_err(|error| ArrowError::InvalidArgumentError(error.to_string()))
            }

            fn supports_data_type(&self, data_type: &ArrowDataType) -> Result<(), ArrowError> {
                if self.id().arrow_extension_name().is_none() {
                    return Err(unsupported(Self::NAME, data_type));
                }
                match recognized(Self::NAME, Some(&self.extension_json()), data_type)? {
                    Some(dtype) if dtype.$parameters() == Some(*self) => Ok(()),
                    _ => Err(unsupported(Self::NAME, data_type)),
                }
            }

            fn try_new(
                data_type: &ArrowDataType,
                leaf: Self::Metadata,
            ) -> Result<Self, ArrowError> {
                leaf.supports_data_type(data_type).map(|()| leaf)
            }
        }
    };
}

leaf_extension!(StringType, string_parameters, crate::STRING_EXTENSION_NAME);
leaf_extension!(BytesType, bytes_parameters, crate::BYTES_EXTENSION_NAME);
