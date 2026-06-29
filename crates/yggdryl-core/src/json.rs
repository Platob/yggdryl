//! JSON serialization via the [`Jsonable`] trait and its global [`JsonParams`].
//!
//! Every value type's JSON form is derived from one binary form: [`to_bson`] renders
//! the value to JSON text (honouring the format) and encodes it to bytes with a
//! [`Charset`], and [`to_json`] is simply those bytes decoded back to a `String`.
//! The format and charset come from a process-global [`JsonParams`]; set it once
//! with [`set_json_params`] to switch all JSON output at once, and
//! [`reset_json_params`] restores the default (compact, UTF-8).
//!
//! [`to_bson`]: Jsonable::to_bson
//! [`to_json`]: Jsonable::to_json

use std::sync::RwLock;

use crate::charset::Charset;
use crate::error::JsonError;

/// Parameters controlling how the crate renders JSON: the text format (compact vs
/// pretty-printed) and the [`Charset`] the bytes are (de)coded with.
///
/// ```
/// use yggdryl_core::{json_params, set_json_params, reset_json_params, Charset, JsonParams};
///
/// set_json_params(JsonParams::pretty().with_indent(4).with_charset(Charset::Ascii));
/// assert!(json_params().is_pretty());
/// assert_eq!(json_params().indent(), 4);
/// assert_eq!(json_params().charset(), Charset::Ascii);
/// reset_json_params();
/// assert_eq!(json_params(), JsonParams::default());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct JsonParams {
    pretty: bool,
    indent: usize,
    charset: Charset,
}

impl JsonParams {
    /// The default: compact output, UTF-8 charset.
    pub const DEFAULT: JsonParams = JsonParams {
        pretty: false,
        indent: 2,
        charset: Charset::Utf8,
    };

    /// Compact output with the default UTF-8 charset (the default).
    pub fn compact() -> Self {
        Self::DEFAULT
    }

    /// Pretty-printed output with the default 2-space indent and UTF-8 charset.
    pub fn pretty() -> Self {
        JsonParams {
            pretty: true,
            ..Self::DEFAULT
        }
    }

    /// Whether output is pretty-printed.
    pub fn is_pretty(&self) -> bool {
        self.pretty
    }

    /// The number of spaces per indent level when pretty-printed.
    pub fn indent(&self) -> usize {
        self.indent
    }

    /// The charset JSON bytes are encoded to and decoded from.
    pub fn charset(&self) -> Charset {
        self.charset
    }

    /// Returns a copy with a different pretty flag.
    pub fn with_pretty(&self, pretty: bool) -> Self {
        JsonParams { pretty, ..*self }
    }

    /// Returns a copy with a different indent width.
    pub fn with_indent(&self, indent: usize) -> Self {
        JsonParams { indent, ..*self }
    }

    /// Returns a copy with a different charset.
    pub fn with_charset(&self, charset: Charset) -> Self {
        JsonParams { charset, ..*self }
    }
}

impl Default for JsonParams {
    fn default() -> Self {
        Self::DEFAULT
    }
}

static PARAMS: RwLock<JsonParams> = RwLock::new(JsonParams::DEFAULT);

/// The current global [`JsonParams`].
pub fn json_params() -> JsonParams {
    *PARAMS.read().expect("json params lock poisoned")
}

/// Sets the global [`JsonParams`] used by every [`Jsonable`] method.
pub fn set_json_params(params: JsonParams) {
    crate::log_event!(
        info,
        "json params set: pretty={} indent={} charset={}",
        params.pretty,
        params.indent,
        params.charset.name()
    );
    *PARAMS.write().expect("json params lock poisoned") = params;
}

/// Resets the global [`JsonParams`] to [`JsonParams::DEFAULT`].
pub fn reset_json_params() {
    crate::log_event!(info, "json params reset");
    *PARAMS.write().expect("json params lock poisoned") = JsonParams::DEFAULT;
}

/// Renders `value` to a JSON string using `params`' format (charset is applied by
/// the caller when turning the string into bytes).
fn render<T: serde::Serialize>(value: &T, params: &JsonParams) -> String {
    if params.pretty {
        let indent = " ".repeat(params.indent);
        let formatter = serde_json::ser::PrettyFormatter::with_indent(indent.as_bytes());
        let mut buffer = Vec::new();
        let mut serializer = serde_json::Serializer::with_formatter(&mut buffer, formatter);
        value
            .serialize(&mut serializer)
            .expect("value serializes to JSON");
        String::from_utf8(buffer).expect("JSON is valid UTF-8")
    } else {
        serde_json::to_string(value).expect("value serializes to JSON")
    }
}

/// JSON serialization shared by every value type.
///
/// The byte form ([`to_bson`](Jsonable::to_bson) / [`from_bson`](Jsonable::from_bson))
/// is the primitive: JSON text encoded to bytes with the active [`Charset`]. The
/// string form ([`to_json`](Jsonable::to_json) / [`from_json`](Jsonable::from_json))
/// is derived from it. A type only has to be `serde`-serializable; the four methods
/// and the [`params`](Jsonable::params) lookup all have defaults, so implementing it
/// is a one-line `impl Jsonable for T {}`.
pub trait Jsonable: serde::Serialize + serde::de::DeserializeOwned {
    /// The JSON parameters (format + charset) this type uses. Defaults to the
    /// process-global [`json_params`]; override to pin a type to fixed params.
    fn params() -> JsonParams {
        json_params()
    }

    /// The value rendered to JSON text and encoded to bytes with the charset — the
    /// canonical "binary JSON" form. ([`to_json`](Jsonable::to_json) is these bytes
    /// decoded back to a `String`.)
    fn to_bson(&self) -> Vec<u8> {
        let params = Self::params();
        params.charset().encode(&render(self, &params))
    }

    /// Reconstructs a value from the bytes produced by [`to_bson`](Jsonable::to_bson).
    fn from_bson(bytes: &[u8]) -> Result<Self, JsonError> {
        let text = Self::params().charset().decode(bytes)?;
        serde_json::from_str(&text).map_err(|err| JsonError::Parse(err.to_string()))
    }

    /// The JSON string form (the [`to_bson`](Jsonable::to_bson) bytes decoded with
    /// the charset).
    fn to_json(&self) -> String {
        Self::params()
            .charset()
            .decode(&self.to_bson())
            .expect("rendered JSON round-trips through its own charset")
    }

    /// Parses the JSON string form (encoding it with the charset, then
    /// [`from_bson`](Jsonable::from_bson)).
    fn from_json(value: &str) -> Result<Self, JsonError> {
        Self::from_bson(&Self::params().charset().encode(value))
    }
}
