//! Options for the XML record encoding.

use smol_str::SmolStr;

#[cfg(feature = "arrow")]
use crate::media::IORecordOptions;
use crate::text::Formatting;
use crate::{DataType, Level, Metadata, Result};

/// The document element a write creates when none is declared.
pub const DEFAULT_DOCUMENT_ELEMENT: &str = "rows";

/// Settings for XML rows reached through the ordinary record-media methods.
///
/// Two names describe the wire and nothing else: `root` is the document
/// element a write creates, and `row` is the element one row is written as and
/// read from. Both are optional because a read does not need them - an
/// undeclared `root` is whatever the document already has, and an undeclared
/// `row` is the first element under it, so a document written elsewhere reads
/// without configuration - while a write falls back to
/// [`DEFAULT_DOCUMENT_ELEMENT`] and
/// [`DEFAULT_ROOT_NAME`](crate::media::DEFAULT_ROOT_NAME).
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct XmlOptions {
    /// Root Field name; [`DEFAULT_ROOT_NAME`](crate::media::DEFAULT_ROOT_NAME) unless set.
    pub name: SmolStr,
    /// Declared root datatype; inferred from the document's rows when absent.
    pub dtype: Option<DataType>,
    /// Root metadata; empty unless declared.
    pub metadata: Metadata,
    /// Whether a cast may null a value it cannot convert.
    pub safe: bool,
    /// Rows per batch a reader yields.
    pub batch_row_size: Option<usize>,
    /// Most result rows in total.
    pub max_row_size: Option<u64>,
    /// Most Arrow in-memory bytes of result rows.
    pub max_byte_size: Option<u64>,
    /// Rows published per streamed-write commit; `None` publishes once.
    pub commit_row_size: Option<usize>,
    /// Compression level applied when the handle declares a coding.
    pub level: Level,
    /// Column names forming a write's match key; empty means overwrite.
    pub merge_by_names: Vec<String>,
    /// Column names a read or write is narrowed to; empty selects everything.
    pub select_by_names: Vec<String>,
    /// Partition equalities a read is pruned and filtered by; empty keeps all.
    pub filter_partitions: Vec<(String, String)>,
    root: Option<SmolStr>,
    row: Option<SmolStr>,
    formatting: Formatting,
}

impl XmlOptions {
    /// Build the default XML record options.
    #[must_use]
    pub fn new() -> Self {
        Self {
            name: SmolStr::new_static(crate::media::DEFAULT_ROOT_NAME),
            dtype: None,
            metadata: Metadata::new(),
            safe: false,
            batch_row_size: None,
            max_row_size: None,
            max_byte_size: None,
            commit_row_size: None,
            level: Level::DEFAULT,
            merge_by_names: Vec::new(),
            select_by_names: Vec::new(),
            filter_partitions: Vec::new(),
            root: None,
            row: None,
            formatting: Formatting::new(),
        }
    }

    /// Borrow the declared document element name.
    #[must_use]
    pub fn root(&self) -> Option<&str> {
        self.root.as_deref()
    }

    /// Set or clear the document element name.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is not an XML name, leaving these
    /// options unchanged.
    pub fn set_root(&mut self, root: Option<&str>) -> Result<()> {
        self.root = checked(root)?;
        Ok(())
    }

    /// Return these options with a declared document element name.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is not an XML name.
    pub fn with_root(mut self, root: &str) -> Result<Self> {
        self.set_root(Some(root))?;
        Ok(self)
    }

    /// Borrow the declared row element name.
    #[must_use]
    pub fn row(&self) -> Option<&str> {
        self.row.as_deref()
    }

    /// Set or clear the row element name.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is not an XML name, leaving these
    /// options unchanged.
    pub fn set_row(&mut self, row: Option<&str>) -> Result<()> {
        self.row = checked(row)?;
        Ok(())
    }

    /// Return these options with a declared row element name.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is not an XML name.
    pub fn with_row(mut self, row: &str) -> Result<Self> {
        self.set_row(Some(row))?;
        Ok(self)
    }

    /// Return the document element a write creates.
    #[must_use]
    pub fn write_root(&self) -> &str {
        self.root().unwrap_or(DEFAULT_DOCUMENT_ELEMENT)
    }

    /// Return the element one written row is spelled as.
    #[must_use]
    pub fn write_row(&self) -> &str {
        self.row().unwrap_or(crate::media::DEFAULT_ROOT_NAME)
    }

    /// Return how written rows are laid out.
    ///
    /// [`Indent::Default`](crate::text::Indent::Default) puts each row element
    /// on its own line indented one level and keeps the row's own children on
    /// that line, which is what a data file wants; an explicit width indents
    /// the children too, and [`Indent::None`](crate::text::Indent::None) writes
    /// the whole document without a byte of layout.
    #[must_use]
    pub const fn formatting(&self) -> Formatting {
        self.formatting
    }

    /// Set how written rows are laid out.
    pub const fn set_formatting(&mut self, formatting: Formatting) {
        self.formatting = formatting;
    }

    /// Return these options with a different row layout.
    #[must_use]
    pub const fn with_formatting(mut self, formatting: Formatting) -> Self {
        self.set_formatting(formatting);
        self
    }
}

/// Validate one declared element name.
fn checked(name: Option<&str>) -> Result<Option<SmolStr>> {
    let Some(name) = name else {
        return Ok(None);
    };
    crate::text::xml::wire::check_name(name)?;
    Ok(Some(SmolStr::new(name)))
}

impl Default for XmlOptions {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "arrow")]
impl IORecordOptions for XmlOptions {
    crate::record_options_fields!();
}

#[cfg(feature = "arrow")]
impl From<XmlOptions> for crate::media::RecordOptions {
    fn from(value: XmlOptions) -> Self {
        Self::Xml(Box::new(value))
    }
}
