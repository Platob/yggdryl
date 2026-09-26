//! The settings an XMLA document read or write takes.

use smol_str::SmolStr;

use crate::media::IORecordOptions;
use crate::{Field, Filter, Level, Selector};

use super::vocabulary::{Content, Method};

/// The settings an XMLA rowset document is read and written with.
///
/// The shared settings are every record encoding's. XMLA adds what the
/// document states about itself: whether the rowset travels inside a SOAP
/// envelope or as a bare `root`, which method's response the envelope
/// carries, and which of the schema and the rows the document holds.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct XmlaOptions {
    /// Root Field name; the declared field's when one is declared.
    pub name: SmolStr,
    /// The declared root; `None` infers the shape.
    pub field: Option<Field>,
    /// The rows a read or write keeps.
    pub filter: Filter,
    /// The columns a read or write publishes.
    pub select: Selector,
    /// The columns forming an explicit merge's match key.
    pub merge_by: Selector,
    /// Whether a cast may null a value it cannot convert.
    pub safe: bool,
    /// Bytes per batch, whichever of this and `batch_row_size` binds first.
    pub batch_byte_size: Option<u64>,
    /// Rows per batch, when a reader should bound them.
    pub batch_row_size: Option<usize>,
    /// Most result rows in total - a count of rows, not a per-row byte cap.
    pub max_row_size: Option<u64>,
    /// Most Arrow in-memory bytes of result rows, never encoded bytes.
    pub max_byte_size: Option<u64>,
    /// Rows published per streamed-write commit; `None` publishes once.
    pub commit_row_size: Option<usize>,
    /// Compression level applied when the handle declares a coding.
    pub level: Level,
    /// Whether the document is a SOAP message - the response of `method` -
    /// or the bare rowset `root`. A read accepts either.
    pub envelope: bool,
    /// The method whose response a written envelope carries.
    pub method: Method,
    /// Which of the schema and the rows a written document holds.
    pub content: Content,
}

impl XmlaOptions {
    /// The default options: an `ExecuteResponse` envelope holding the
    /// schema and the rows.
    #[must_use]
    pub fn new() -> Self {
        Self {
            name: SmolStr::new_static(crate::media::DEFAULT_ROOT_NAME),
            field: None,
            filter: Filter::always_true(),
            select: Selector::all(),
            merge_by: Selector::all(),
            safe: false,
            batch_byte_size: None,
            batch_row_size: None,
            max_row_size: None,
            max_byte_size: None,
            commit_row_size: None,
            level: Level::DEFAULT,
            envelope: true,
            method: Method::Execute,
            content: Content::SchemaData,
        }
    }

    /// Return these options writing a bare rowset `root` with no envelope.
    #[must_use]
    pub const fn without_envelope(mut self) -> Self {
        self.envelope = false;
        self
    }

    /// Return these options writing the response of `method`.
    #[must_use]
    pub const fn with_method(mut self, method: Method) -> Self {
        self.method = method;
        self
    }

    /// Return these options writing `content`.
    #[must_use]
    pub const fn with_content(mut self, content: Content) -> Self {
        self.content = content;
        self
    }
}

impl Default for XmlaOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl IORecordOptions for XmlaOptions {
    crate::record_options_fields!();
}
