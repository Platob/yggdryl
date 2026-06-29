//! Global JSON formatting parameters.
//!
//! Every `to_json` in the crate renders through [`render`], which consults a
//! process-global [`JsonFormat`]. Set it once with [`set_json_format`] to switch
//! all JSON output between compact and pretty-printed; [`reset_json_format`]
//! restores the default. Parsing (`from_json`) is unaffected.

use std::sync::RwLock;

/// Parameters controlling how the crate renders JSON.
///
/// ```
/// use yggdryl_core::{json_format, set_json_format, reset_json_format, JsonFormat};
///
/// set_json_format(JsonFormat::pretty().with_indent(4));
/// assert!(json_format().is_pretty());
/// assert_eq!(json_format().indent(), 4);
/// reset_json_format();
/// assert!(!json_format().is_pretty());
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct JsonFormat {
    pretty: bool,
    indent: usize,
}

impl JsonFormat {
    /// The default: compact output (no newlines or indentation).
    pub const DEFAULT: JsonFormat = JsonFormat {
        pretty: false,
        indent: 2,
    };

    /// Compact output (the default).
    pub fn compact() -> Self {
        Self::DEFAULT
    }

    /// Pretty-printed output with the default 2-space indent.
    pub fn pretty() -> Self {
        JsonFormat {
            pretty: true,
            indent: 2,
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

    /// Returns a copy with a different pretty flag.
    pub fn with_pretty(&self, pretty: bool) -> Self {
        JsonFormat { pretty, ..*self }
    }

    /// Returns a copy with a different indent width.
    pub fn with_indent(&self, indent: usize) -> Self {
        JsonFormat { indent, ..*self }
    }
}

impl Default for JsonFormat {
    fn default() -> Self {
        Self::DEFAULT
    }
}

static FORMAT: RwLock<JsonFormat> = RwLock::new(JsonFormat::DEFAULT);

/// The current global [`JsonFormat`].
pub fn json_format() -> JsonFormat {
    *FORMAT.read().expect("json format lock poisoned")
}

/// Sets the global [`JsonFormat`] used by every `to_json`.
pub fn set_json_format(format: JsonFormat) {
    crate::log_event!(
        info,
        "json format set: pretty={} indent={}",
        format.pretty,
        format.indent
    );
    *FORMAT.write().expect("json format lock poisoned") = format;
}

/// Resets the global [`JsonFormat`] to [`JsonFormat::DEFAULT`].
pub fn reset_json_format() {
    crate::log_event!(info, "json format reset");
    *FORMAT.write().expect("json format lock poisoned") = JsonFormat::DEFAULT;
}

/// Renders `value` to a JSON string using the current global [`JsonFormat`].
pub(crate) fn render<T: serde::Serialize>(value: &T) -> String {
    let format = json_format();
    if format.pretty {
        let indent = " ".repeat(format.indent);
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
