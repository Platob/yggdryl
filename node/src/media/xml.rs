//! Native JavaScript redirections over the XML codecs.
//!
//! This boundary owns no XML model: the document walk, the mapping, the
//! escaping and the schema all stay native, and values cross as the shared
//! `Scalar` every other codec crosses as.

use napi::bindgen_prelude::{Buffer, Result};
use napi_derive::napi;
use yggdryl::media::xml;
use yggdryl::text::{Formatting, Indent, Limits};

use crate::exact_u64;
use crate::napi_error;
use crate::text::codec::JsScalar;
use crate::types::field::JsField;

/// Resource limits shared by every XML decode entry point.
#[napi(object)]
pub struct XmlDecodeLimitsInput {
    /// Maximum structural nesting in a document.
    pub max_depth: Option<f64>,
    /// Maximum encoded bytes consumed by one decode.
    pub max_input_bytes: Option<f64>,
    /// Maximum decoded elements and attributes.
    pub max_nodes: Option<f64>,
}

impl XmlDecodeLimitsInput {
    fn into_core(self) -> Result<Limits> {
        let defaults = Limits::default();
        Ok(Limits::new(
            exact_limit(self.max_depth, "maxDepth", defaults.max_depth())?,
            exact_limit(
                self.max_input_bytes,
                "maxInputBytes",
                defaults.max_input_bytes(),
            )?,
            exact_limit(self.max_nodes, "maxNodes", defaults.max_nodes())?,
            defaults.max_documents(),
        ))
    }
}

fn decode_limits(value: Option<XmlDecodeLimitsInput>) -> Result<Limits> {
    value.map_or_else(|| Ok(Limits::default()), XmlDecodeLimitsInput::into_core)
}

fn exact_limit(value: Option<f64>, name: &str, default: usize) -> Result<usize> {
    value.map_or(Ok(default), |value| {
        usize::try_from(exact_u64(value, name)?).map_err(|_| {
            napi::Error::from_reason(format!("{name} does not fit this platform's usize"))
        })
    })
}

/// The indentation a write renders with.
fn formatting(indent: Option<f64>) -> Result<Formatting> {
    let indent = match indent {
        None => Indent::Default,
        Some(width) => {
            let width = exact_u64(width, "indent")?;
            if width == 0 {
                Indent::None
            } else {
                Indent::Spaces(u8::try_from(width).map_err(|_| {
                    napi::Error::from_reason("indent must be at most 255 spaces".to_owned())
                })?)
            }
        }
    };
    Ok(Formatting::new().with_indent(indent))
}

/// Decode one XML document into the shared native `Scalar`.
#[napi(js_name = "xmlLoadsNative", skip_typescript)]
pub fn xml_loads_native(input: Buffer, limits: Option<XmlDecodeLimitsInput>) -> Result<JsScalar> {
    let limits = decode_limits(limits)?;
    xml::from_bytes_with_limits(&input, limits)
        .map(JsScalar::from_core)
        .map_err(napi_error)
}

/// Encode one native `Scalar` as a whole XML document rooted at `name`.
#[napi(js_name = "xmlDumpsNative", skip_typescript)]
pub fn xml_dumps_native(value: &JsScalar, name: String, indent: Option<f64>) -> Result<Buffer> {
    let formatting = formatting(indent)?;
    xml::into_bytes_with_formatting(&name, &value.inner, formatting)
        .map(Buffer::from)
        .map_err(napi_error)
}

/// Decode one XML document under a field, as the typed value it declares.
#[napi(js_name = "xmlLoadsWithFieldNative", skip_typescript)]
pub fn xml_loads_with_field_native(
    input: Buffer,
    field: &JsField,
    limits: Option<XmlDecodeLimitsInput>,
) -> Result<JsScalar> {
    let limits = decode_limits(limits)?;
    xml::from_bytes_with_limits(&input, limits)
        .and_then(|value| field.inner.from_natural_value(value))
        .map(JsScalar::from_core)
        .map_err(napi_error)
}

/// Read an XML Schema document as the field it declares.
#[napi(js_name = "xmlSchemaNative", skip_typescript)]
pub fn xml_schema_native(
    input: Buffer,
    root: Option<String>,
    limits: Option<XmlDecodeLimitsInput>,
) -> Result<JsField> {
    let limits = decode_limits(limits)?;
    xml::field_from_xsd(&input, limits, root.as_deref())
        .map(JsField::from_core)
        .map_err(napi_error)
}

/// Write one field as the XML Schema that declares it.
#[napi(js_name = "xmlSchemaDumpsNative", skip_typescript)]
pub fn xml_schema_dumps_native(field: &JsField, indent: Option<f64>) -> Result<Buffer> {
    let formatting = formatting(indent)?;
    xml::field_into_xsd(&field.inner, formatting)
        .map(Buffer::from)
        .map_err(napi_error)
}
