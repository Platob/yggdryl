//! The settings an XML record read or write takes.

use smol_str::SmolStr;

use crate::media::IORecordOptions;
use crate::text::{Formatting, Limits};

/// The name a written document's root element takes when none is declared.
pub const DEFAULT_DOCUMENT_NAME: &str = "rows";

/// The settings an XML record read or write takes.
///
/// XML adds four settings to the shared surface. Three say what a document
/// looks like rather than what a row means - the wrapper's name, the row
/// element's name, and the indentation - and the fourth bounds what a hostile
/// document may cost. Everything else about the mapping is fixed, because a
/// knob there would be a second answer to what a document says.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct XmlOptions {
    /// Root Field name; the declared field's when one is declared.
    pub name: SmolStr,
    /// The declared root; `None` infers the shape.
    pub field: Option<crate::Field>,
    /// The rows a read or write keeps.
    pub filter: crate::Filter,
    /// The columns a read or write publishes.
    pub select: crate::Selector,
    /// The columns forming an explicit merge's match key.
    pub merge_by: crate::Selector,
    /// Whether a cast may null a value it cannot convert.
    pub safe: bool,
    /// Bytes per batch, whichever of this and `batch_row_size` binds first.
    pub batch_byte_size: Option<u64>,
    /// Rows per batch a reader yields.
    pub batch_row_size: Option<usize>,
    /// Most result rows in total.
    pub max_row_size: Option<u64>,
    /// Most Arrow in-memory bytes of result rows.
    pub max_byte_size: Option<u64>,
    /// Rows published per streamed-write commit; `None` publishes once.
    pub commit_row_size: Option<usize>,
    /// Compression level for the handle's own content coding.
    pub level: crate::Level,
    /// The document element a write wraps its rows in.
    pub document: SmolStr,
    /// The element a row is written as, and read from.
    ///
    /// `None` reads the name every row element agrees on and refuses a
    /// document whose children disagree; naming it here reads a wrapper that
    /// holds more than rows. A write uses the declared field's name.
    pub row_element: Option<SmolStr>,
    /// How much of a document a read will decode before refusing it.
    pub limits: Limits,
    /// The indentation a write uses. The level rides on `level`.
    pub indent: crate::text::Indent,
}

impl XmlOptions {
    /// Build the default XML options.
    pub fn new() -> Self {
        Self {
            name: SmolStr::new_static(crate::media::DEFAULT_ROOT_NAME),
            field: None,
            filter: crate::Filter::always_true(),
            select: crate::Selector::all(),
            merge_by: crate::Selector::all(),
            safe: false,
            batch_byte_size: None,
            batch_row_size: None,
            max_row_size: None,
            max_byte_size: None,
            commit_row_size: None,
            level: crate::Level::DEFAULT,
            document: SmolStr::new_static(DEFAULT_DOCUMENT_NAME),
            row_element: None,
            limits: Limits::default(),
            indent: crate::text::Indent::Default,
        }
    }

    /// Return these options with a different document element name.
    #[must_use]
    pub fn with_document(mut self, document: impl Into<SmolStr>) -> Self {
        self.document = document.into();
        self
    }

    /// Return these options reading one named row element.
    #[must_use]
    pub fn with_row_element(mut self, row: impl Into<SmolStr>) -> Self {
        self.row_element = Some(row.into());
        self
    }

    /// Return these options with different decode limits.
    #[must_use]
    pub const fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /// Return these options with a different indentation.
    #[must_use]
    pub const fn with_indent(mut self, indent: crate::text::Indent) -> Self {
        self.indent = indent;
        self
    }

    /// The formatting a write renders with.
    pub(crate) fn formatting(&self) -> Formatting {
        Formatting::new()
            .with_indent(self.indent)
            .with_level(self.level)
    }
}

impl Default for XmlOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl IORecordOptions for XmlOptions {
    crate::record_options_fields!();
}
