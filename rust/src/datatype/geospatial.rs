//! The geospatial pair: geometry and geography, and the value they share.

use std::sync::Arc;

use smol_str::SmolStr;

use crate::enums::EdgeAlgorithm;
use crate::{DataType, Error, Result};

/// The coordinate reference system both formats fill when none is given.
///
/// `OGC:CRS84` is longitude/latitude on WGS 84 - the default Parquet's
/// `GEOMETRY`/`GEOGRAPHY` logical types and Iceberg v3's geospatial types
/// share, so it is the one this workspace fills too.
pub const DEFAULT_CRS: &str = "OGC:CRS84";

/// The parameters a geometry or geography column carries.
///
/// One shared value beside [`crate::MapType`], [`crate::DictionaryType`], and
/// [`crate::RunEndEncodedType`]: the coordinate reference system both carry,
/// and the edge algorithm only a geography has - a geometry connects vertices
/// with straight planar lines, so a geometry given an algorithm is refused by
/// name, and a geography given none fills [`EdgeAlgorithm::Spherical`], the
/// default Parquet and Iceberg share.
#[derive(Clone, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
pub struct GeospatialType {
    /// The coordinate reference system, `OGC:CRS84` filled when none is given.
    crs: SmolStr,
    /// The edge interpolation, present exactly on a geography.
    algorithm: Option<EdgeAlgorithm>,
}

impl GeospatialType {
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

impl GeospatialType {
    /// Renders the GeoArrow extension metadata document this type projects.
    ///
    /// A geometry writes `{"crs": <crs>}`; a geography adds its edge
    /// algorithm as `"edges"`. The CRS is always written, defaults included,
    /// so the projected document never depends on what the reader would fill.
    pub(crate) fn to_geoarrow_json(&self) -> String {
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

/// The Arrow extension name and metadata one of the three extension-typed
/// variants projects, `None` for every other datatype.
pub(crate) fn arrow_extension_parts(data_type: &DataType) -> Option<(&'static str, String)> {
    match data_type {
        DataType::Variant => Some((VARIANT_EXTENSION_NAME, String::new())),
        DataType::Geometry(geospatial) | DataType::Geography(geospatial) => {
            Some((GEOARROW_WKB_EXTENSION_NAME, geospatial.to_geoarrow_json()))
        }
        _ => None,
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
    /// Creates the self-describing semi-structured Variant type.
    ///
    /// It takes no parameters - shredding is a physical layout, not part of
    /// the logical type - and a variant *value* is a [`crate::Value`]: a
    /// self-describing tree, so the binary form is an encoding of the one
    /// value model, never a second one. The finite union of declared members
    /// is [`Self::dense_union`]; in the grammar the parenthesis
    /// disambiguates, so bare `variant` parses to this type and
    /// `variant(...)` stays the union sugar.
    #[must_use]
    pub const fn variant() -> Self {
        Self::Variant
    }

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
        Ok(Self::Geometry(Arc::new(GeospatialType::geometry(crs)?)))
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
        Ok(Self::Geography(Arc::new(GeospatialType::geography(
            crs, algorithm,
        )?)))
    }
}
