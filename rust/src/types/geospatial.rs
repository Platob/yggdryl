//! Geometry and geography datatypes.

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use smol_str::{SmolStr, format_smolstr};

use crate::types::parser::Parser;
use crate::{DataType, DataTypeId, EdgeAlgorithm, Error, Result, Scalar, Value};

/// Arrow casts owned by this datatype family.
pub(crate) mod casts {
    use std::sync::Arc;

    use arrow_array::{Array, ArrayRef, BinaryArray, LargeStringArray, StringArray, StringViewArray};
    use arrow_buffer::BooleanBuffer;
    use arrow_schema::DataType as ArrowDataType;

    use crate::types::wkb;
    use crate::Field;
    use crate::arrow::{Error, Result};
    use crate::types::budget::{MaterializationBudget, reserve_vec_bytes};
    use crate::types::cast::{downcast, internal_target_error};
    use crate::types::cast::columns::is_exposed;

    /// Validates every exposed, non-null payload of a Binary array as WKB on its
    /// way into a geospatial field, naming the field, the row, and the byte
    /// position the reader stopped at.
    ///
    /// The streaming bounds scan walks the whole payload without materializing a
    /// geometry, so validation holds nothing per row.
    pub(crate) fn validate_wkb_ingest(
        array: &dyn Array,
        field: &Field,
        exposure: Option<&BooleanBuffer>,
    ) -> Result<()> {
        let source = downcast::<BinaryArray>(array)?;
        for index in 0..source.len() {
            if !is_exposed(exposure, index) || source.is_null(index) {
                continue;
            }
            wkb::bounding_box(source.value(index)).map_err(|error| {
                Error::IncompatibleSchema(format!(
                    "field {:?} row {index}: expected WKB bytes for a {} value, got {error}",
                    field.name(),
                    field.dtype().name(),
                ))
            })?;
        }
        Ok(())
    }

    /// Renders a recognized geospatial Binary column as WKT text.
    ///
    /// Two passes keep the materialization honest: the first parses and renders
    /// each exposed value once to count the exact output payload against the
    /// budget - holding one rendered value at a time - and the second builds the
    /// target text array under that reservation.
    pub(crate) fn render_wkt_array(
        array: &ArrayRef,
        expected: &ArrowDataType,
        field: &Field,
        exposure: Option<&BooleanBuffer>,
        budget: &mut MaterializationBudget,
    ) -> Result<ArrayRef> {
        let source = downcast::<BinaryArray>(array.as_ref())?;
        let mut total = 0usize;
        let mut maximum = 0usize;
        for index in 0..source.len() {
            if !is_exposed(exposure, index) || source.is_null(index) {
                continue;
            }
            let text = wkt_for_cell(field, index, source.value(index))?;
            total = total.checked_add(text.len()).ok_or_else(|| {
                Error::IncompatibleSchema("WKT output payload exceeds usize".to_owned())
            })?;
            maximum = maximum.max(text.len());
        }
        budget.add_array(field.dtype(), source.len())?;
        budget.add_bytes(total)?;
        // View outputs keep the largest logical value alive while the final view
        // buffers are appended.
        if matches!(expected, ArrowDataType::Utf8View) {
            budget.add_bytes(maximum)?;
        }
        let mut rendered: Vec<Option<String>> = Vec::new();
        reserve_vec_bytes::<Option<String>>(budget, source.len())?;
        rendered.try_reserve_exact(source.len()).map_err(|error| {
            Error::IncompatibleSchema(format!("WKT output allocation failed: {error}"))
        })?;
        for index in 0..source.len() {
            if !is_exposed(exposure, index) || source.is_null(index) {
                rendered.push(None);
            } else {
                rendered.push(Some(wkt_for_cell(field, index, source.value(index))?));
            }
        }
        Ok(match expected {
            ArrowDataType::Utf8 => Arc::new(rendered.into_iter().collect::<StringArray>()) as ArrayRef,
            ArrowDataType::LargeUtf8 => {
                Arc::new(rendered.into_iter().collect::<LargeStringArray>()) as ArrayRef
            }
            ArrowDataType::Utf8View => {
                Arc::new(rendered.into_iter().collect::<StringViewArray>()) as ArrayRef
            }
            _ => return Err(internal_target_error("geospatial WKT")),
        })
    }

    /// Renders one WKB cell as WKT, naming the field and row when the bytes are
    /// not one well-formed geometry.
    fn wkt_for_cell(field: &Field, index: usize, bytes: &[u8]) -> Result<String> {
        wkb::into_wkt(bytes).map_err(|error| {
            Error::IncompatibleSchema(format!(
                "field {:?} row {index}: expected WKB bytes to render as WKT, got {error}",
                field.name(),
            ))
        })
    }
}

// ------------------------------------------------------------------------
// The geospatial pair: geometry and geography, and the value they share.
// ------------------------------------------------------------------------

/// The coordinate reference system both formats fill when none is given.
///
/// `OGC:CRS84` is longitude/latitude on WGS 84 - the default Parquet's
/// `GEOMETRY`/`GEOGRAPHY` logical types and Iceberg v3's geospatial types
/// share, so it is the one this workspace fills too.
pub(crate) const DEFAULT_CRS: &str = "OGC:CRS84";

/// One geospatial datatype and its validated parameters.
#[derive(Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum GeospatialType {
    /// Planar geometry.
    Geometry(Arc<GeospatialParameters>),
    /// Spherical or spheroidal geography.
    Geography(Arc<GeospatialParameters>),
}

impl GeospatialType {
    /// Return the exact datatype identifier.
    pub const fn id(&self) -> DataTypeId {
        match self {
            Self::Geometry(_) => DataTypeId::Geometry,
            Self::Geography(_) => DataTypeId::Geography,
        }
    }
}

impl From<GeospatialType> for DataType {
    fn from(value: GeospatialType) -> Self {
        match value {
            GeospatialType::Geometry(parameters) => Self::Geometry(parameters),
            GeospatialType::Geography(parameters) => Self::Geography(parameters),
        }
    }
}

impl TryFrom<&DataType> for GeospatialType {
    type Error = Error;

    fn try_from(value: &DataType) -> Result<Self> {
        match value {
            DataType::Geometry(parameters) => Ok(Self::Geometry(Arc::clone(parameters))),
            DataType::Geography(parameters) => Ok(Self::Geography(Arc::clone(parameters))),
            other => Err(Error::InvalidDataType {
                kind: "geospatial",
                reason: SmolStr::new(format!("expected a geospatial datatype, got {other}")),
            }),
        }
    }
}

/// The parameters a geometry or geography column carries.
///
/// One shared value beside [`crate::MapType`], [`crate::DictionaryType`], and
/// [`crate::RunEndEncodedType`]: the coordinate reference system both carry,
/// and the edge algorithm only a geography has - a geometry connects vertices
/// with straight planar lines, so a geometry given an algorithm is refused by
/// name, and a geography given none fills [`EdgeAlgorithm::Spherical`], the
/// default Parquet and Iceberg share.
#[derive(Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct GeospatialParameters {
    /// The coordinate reference system, `OGC:CRS84` filled when none is given.
    crs: SmolStr,
    /// The edge interpolation, present exactly on a geography.
    algorithm: Option<EdgeAlgorithm>,
}

impl GeospatialParameters {
    /// The parameters of a geometry: a CRS and no edge algorithm.
    ///
    /// # Errors
    ///
    /// Returns an error when `crs` is empty - an empty reference system names
    /// nothing, and the absent spelling is `None`, which fills the default.
    pub fn geometry(crs: Option<&str>) -> Result<Self> {
        Ok(Self {
            crs: validated_crs(crs)?,
            algorithm: None,
        })
    }

    /// The parameters of a geography: a CRS and an edge algorithm.
    ///
    /// A geography given no algorithm fills [`EdgeAlgorithm::Spherical`] - the
    /// default both Parquet and Iceberg fill - so the field is never absent on
    /// a geography and never present on a geometry.
    ///
    /// # Errors
    ///
    /// Returns an error when `crs` is empty.
    pub fn geography(crs: Option<&str>, algorithm: Option<EdgeAlgorithm>) -> Result<Self> {
        Ok(Self {
            crs: validated_crs(crs)?,
            algorithm: Some(algorithm.unwrap_or_default()),
        })
    }

    /// The coordinate reference system, never empty.
    pub fn crs(&self) -> &str {
        &self.crs
    }

    /// Return whether the CRS is the `OGC:CRS84` default both formats fill.
    pub fn has_default_crs(&self) -> bool {
        self.crs == DEFAULT_CRS
    }

    /// The edge algorithm, present exactly when this describes a geography.
    pub const fn algorithm(&self) -> Option<EdgeAlgorithm> {
        self.algorithm
    }
}

/// The canonical Arrow extension name of the variant type.
///
/// The storage is a struct of a non-nullable `metadata` Binary and a
/// non-nullable `value` Binary, and the extension metadata is the empty
/// string, exactly as the canonical `arrow.parquet.variant` extension spells
/// them.
pub(crate) const VARIANT_EXTENSION_NAME: &str = "arrow.parquet.variant";

/// The community GeoArrow extension name of the geospatial pair.
///
/// The storage is a Binary column of WKB payloads and the extension metadata
/// is the GeoArrow JSON document: `{"crs": <crs>}` for a geometry and
/// `{"crs": <crs>, "edges": "<algorithm>"}` for a geography. GeoArrow is a
/// community specification whose own documents say it is not finalized, so
/// this spelling is revisitable if the published one moves.
pub(crate) const GEOARROW_WKB_EXTENSION_NAME: &str = "geoarrow.wkb";

impl GeospatialParameters {
    /// Renders the GeoArrow extension metadata document this type projects.
    ///
    /// A geometry writes `{"crs": <crs>}`; a geography adds its edge
    /// algorithm as `"edges"`. The CRS is always written, defaults included,
    /// so the projected document never depends on what the reader would fill.
    pub(crate) fn geoarrow_json(&self) -> String {
        let mut document = serde_json::Map::with_capacity(2);
        document.insert(
            "crs".to_owned(),
            serde_json::Value::String(self.crs.to_string()),
        );
        if let Some(algorithm) = self.algorithm {
            document.insert(
                "edges".to_owned(),
                serde_json::Value::String(algorithm.as_str().to_owned()),
            );
        }
        serde_json::Value::Object(document).to_string()
    }

    /// Parses a GeoArrow extension metadata document back into parameters.
    ///
    /// An absent or empty document is a geometry with the default CRS. A
    /// present `"edges"` string selects a geography and its algorithm; an
    /// absent one a geometry, exactly the distinction the projection writes.
    /// A `"crs"` that is absent or `null` fills the shared default.
    ///
    /// # Errors
    ///
    /// Returns an error when the document is not a JSON object, when `"crs"`
    /// or `"edges"` holds something other than a string, or when the edge
    /// algorithm is not in the shared vocabulary.
    pub(crate) fn from_geoarrow_json(document: Option<&str>) -> Result<Self> {
        let text = document.unwrap_or("").trim();
        if text.is_empty() {
            return Self::geometry(None);
        }
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|error| Error::InvalidDataType {
                kind: "geospatial",
                reason: SmolStr::new(format!(
                    "expected a GeoArrow JSON metadata object, got unparsable JSON: {error}"
                )),
            })?;
        let Some(object) = value.as_object() else {
            return Err(geoarrow_metadata_error(format!(
                "expected a GeoArrow JSON metadata object, got {}",
                crate::text::elide_display(&value)
            )));
        };
        let crs = match object.get("crs") {
            None | Some(serde_json::Value::Null) => None,
            Some(serde_json::Value::String(crs)) => Some(crs.as_str()),
            Some(other) => {
                return Err(geoarrow_metadata_error(format!(
                    "expected a JSON string \"crs\", got {}",
                    crate::text::elide_display(other)
                )));
            }
        };
        match object.get("edges") {
            None | Some(serde_json::Value::Null) => Self::geometry(crs),
            Some(serde_json::Value::String(edges)) => {
                Self::geography(crs, Some(EdgeAlgorithm::from_str(edges)?))
            }
            Some(other) => Err(geoarrow_metadata_error(format!(
                "expected a JSON string \"edges\", got {}",
                crate::text::elide_display(other)
            ))),
        }
    }
}

fn geoarrow_metadata_error(reason: String) -> Error {
    Error::InvalidDataType {
        kind: "geospatial",
        reason: SmolStr::new(reason),
    }
}

/// Fill or validate the CRS: `None` is the shared default, empty is refused.
fn validated_crs(crs: Option<&str>) -> Result<SmolStr> {
    match crs {
        None => Ok(SmolStr::new_static(DEFAULT_CRS)),
        Some("") => Err(Error::InvalidDataType {
            kind: "geospatial",
            reason: SmolStr::new_static(
                "expected a coordinate reference system name, got an empty string; \
                 omit it to fill the OGC:CRS84 default",
            ),
        }),
        Some(crs) => Ok(SmolStr::new(crs)),
    }
}

impl DataType {
    /// Creates a geometry: planar geospatial features as Well-Known Binary.
    ///
    /// `None` fills the `OGC:CRS84` default. A geometry has no edge
    /// algorithm - straight planar lines need none - so there is nothing else
    /// to give; [`Self::geography`] is the type that takes one.
    ///
    /// # Errors
    ///
    /// Returns an error when `crs` is empty.
    pub fn geometry(crs: Option<&str>) -> Result<Self> {
        Ok(Self::Geometry(Arc::new(GeospatialParameters::geometry(
            crs,
        )?)))
    }

    /// Creates a geography: geospatial features on a sphere or spheroid.
    ///
    /// `None` fills the `OGC:CRS84` default and the
    /// [`EdgeAlgorithm::Spherical`] default, so `geography(None, None)` is
    /// the type both formats spell bare.
    ///
    /// # Errors
    ///
    /// Returns an error when `crs` is empty.
    pub fn geography(crs: Option<&str>, algorithm: Option<EdgeAlgorithm>) -> Result<Self> {
        Ok(Self::Geography(Arc::new(GeospatialParameters::geography(
            crs, algorithm,
        )?)))
    }
}

// ------------------------------------------------------------------------
// Geometry and geography field markers.
// ------------------------------------------------------------------------




// ------------------------------------------------------------------------
// Geospatial datatype grammar.
// ------------------------------------------------------------------------

impl Parser<'_> {
    pub(crate) fn parse_geospatial(&mut self, geography: bool) -> Result<DataType> {
        let build = |crs: Option<&str>, algorithm: Option<EdgeAlgorithm>| {
            if geography {
                DataType::geography(crs, algorithm)
            } else {
                DataType::geometry(crs)
            }
        };
        let Some(close) = self.consume_opening() else {
            return build(None, None).map_err(|error| self.error_here(format_smolstr!("{error}")));
        };
        // Empty parentheses are the bare spelling with punctuation.
        if self.consume_symbol(close) {
            return build(None, None).map_err(|error| self.error_here(format_smolstr!("{error}")));
        }
        let crs_position = self.current_position();
        let crs = self.parse_text("a coordinate reference system")?;
        let mut algorithm = None;
        if self.consume_separator() {
            let algorithm_position = self.current_position();
            let name = self.parse_text("an edge algorithm")?;
            if !geography {
                return Err(self.error_at(
                    algorithm_position,
                    format_smolstr!(
                        "expected no edge algorithm for geometry, got {name:?}; geography is the type whose edges take one"
                    ),
                ));
            }
            algorithm =
                Some(EdgeAlgorithm::from_str(&name).map_err(|error| {
                    self.error_at(algorithm_position, format_smolstr!("{error}"))
                })?);
        }
        self.expect_symbol(close)?;
        build(Some(&crs), algorithm)
            .map_err(|error| self.error_at(crs_position, format_smolstr!("{error}")))
    }
}

// ------------------------------------------------------------------------
// Geospatial values and typed scalar aliases.
// ------------------------------------------------------------------------

/// Borrowing access shared by geometry and geography values.
pub trait GeospatialValue: Value {
    /// Borrow the validated Well-Known Binary payload.
    fn as_bytes(&self) -> &[u8];
    /// Borrow the shared storage behind the payload.
    ///
    /// The payload is already validated WKB, so reinterpreting a geometry as
    /// a geography clones this handle rather than copying and re-reading it.
    fn storage(&self) -> &Arc<[u8]>;
}

macro_rules! geospatial_leaf {
    ($name:ident) => {
        #[doc = concat!("One validated `", stringify!($name), "` WKB value.")]
        #[repr(transparent)]
        #[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(Arc<[u8]>);

        impl $name {
            /// Validate and construct a WKB value.
            pub fn new(value: impl Into<Arc<[u8]>>) -> Result<Self> {
                let value = value.into();
                super::wkb::Geometry::from_slice(value.as_ref())?;
                Ok(Self(value))
            }

            /// Borrow the canonical WKB bytes.
            pub fn as_bytes(&self) -> &[u8] {
                self.0.as_ref()
            }

            /// Borrow the shared storage without copying the payload.
            pub fn storage(&self) -> &Arc<[u8]> {
                &self.0
            }

            /// Consume this value and return its shared WKB bytes.
            pub fn into_inner(self) -> Arc<[u8]> {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                for byte in self.as_bytes() {
                    write!(formatter, "{byte:02x}")?;
                }
                Ok(())
            }
        }
    };
}

geospatial_leaf!(Geometry);
geospatial_leaf!(Geography);

const _: () = assert!(std::mem::size_of::<Geometry>() == 16);
const _: () = assert!(std::mem::size_of::<Geography>() == 16);

// Each interpretation is its own family, as every width leaf is: the `Scalar`
// variant holds the leaf directly, so there is no grouping enum to widen into.
// Geometry and geography differ in the coordinate reference they name, not in
// the bytes, so rewriting one as the other shares the storage handle.
macro_rules! geospatial_value {
    ($leaf:ident, $dtype:expr) => {
        impl Value for $leaf {
            fn dtype(&self) -> Result<DataType> {
                $dtype
            }

            fn into_scalar(self) -> Scalar {
                Scalar::$leaf(self)
            }

            fn from_scalar(value: &Scalar) -> Option<&Self> {
                match value {
                    Scalar::$leaf(value) => Some(value),
                    _ => None,
                }
            }
        }

        impl GeospatialValue for $leaf {
            fn as_bytes(&self) -> &[u8] {
                <$leaf>::as_bytes(self)
            }

            fn storage(&self) -> &Arc<[u8]> {
                <$leaf>::storage(self)
            }
        }
    };
}

geospatial_value!(Geometry, DataType::geometry(None));
geospatial_value!(Geography, DataType::geography(None, None));

// ------------------------------------------------------------------------
// Arrow projection: WKB bytes, and the canonical variant storage struct.
// ------------------------------------------------------------------------

mod arrow {
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Fields as ArrowFields};

    use crate::types::VariantType;

    impl VariantType {
        /// The Arrow storage a variant column lays out.
        ///
        /// The canonical `arrow.parquet.variant` struct: a non-nullable
        /// `metadata` binary followed by a non-nullable `value` binary.
        /// Shredding is a physical layout, so nothing else appears here.
        pub(crate) fn arrow_storage() -> ArrowDataType {
            ArrowDataType::Struct(ArrowFields::from(vec![
                ArrowField::new("metadata", ArrowDataType::Binary, false),
                ArrowField::new("value", ArrowDataType::Binary, false),
            ]))
        }
    }

    /// The Arrow storage a geospatial column lays out: Well-Known Binary.
    ///
    /// The `geoarrow.wkb` name and the GeoArrow document ride on the field
    /// beside it, because an Arrow datatype has nowhere to carry them.
    pub(crate) const fn geospatial_arrow_storage() -> ArrowDataType {
        ArrowDataType::Binary
    }

    /// Reports whether an Arrow datatype is exactly the canonical variant
    /// storage: a struct of a non-nullable `metadata` Binary followed by a
    /// non-nullable `value` Binary.
    ///
    /// Child field metadata does not participate - it is transport, not
    /// identity - but the order is fixed because Arrow's own struct casting is
    /// positional and would silently relabel swapped children.
    pub(crate) fn is_variant_storage(dtype: &ArrowDataType) -> bool {
        let ArrowDataType::Struct(fields) = dtype else {
            return false;
        };
        fields.len() == 2
            && fields[0].name() == "metadata"
            && fields[1].name() == "value"
            && fields
                .iter()
                .all(|field| field.data_type() == &ArrowDataType::Binary && !field.is_nullable())
    }
}

pub(crate) use arrow::{geospatial_arrow_storage, is_variant_storage};
