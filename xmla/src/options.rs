//! The settings an XMLA document read or write takes.

use std::str::FromStr;

use smol_str::SmolStr;

use yggdryl::media::{IORecordOptions, RecordOptions, RegisteredOptions};
use yggdryl::{Error, Field, Filter, Level, MimeType, Result, Selector};

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
    /// Leading result rows skipped before `max_row_size` counts.
    pub row_offset: Option<u64>,
    /// Most Arrow in-memory bytes of result rows, never encoded bytes.
    pub max_byte_size: Option<u64>,
    /// Whole batches published per streamed-write commit, never rows; `None`
    /// is the destination's own cadence: a leaf, a folder and an Iceberg
    /// table publish once, after the source ends - the table holding every
    /// partition's rows under the process spill bound until then - an
    /// overwrite's first commit replacing and every later one appending
    /// while every commit of a merge merges by its key; a write session by
    /// [`DEFAULT_COMMIT_BYTE_SIZE`](yggdryl::media::DEFAULT_COMMIT_BYTE_SIZE).
    /// The commits completed before a later failure stay published. The rule
    /// is [`IORecordOptions::commit_batch_num`]'s.
    pub commit_batch_num: Option<usize>,
    /// The threads a write of several parts runs on at once; `None` is the
    /// destination's own answer.
    pub num_threads: Option<usize>,
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
            name: SmolStr::new_static(yggdryl::media::DEFAULT_ROOT_NAME),
            field: None,
            filter: Filter::always_true(),
            select: Selector::all(),
            merge_by: Selector::all(),
            safe: false,
            batch_byte_size: None,
            batch_row_size: None,
            max_row_size: None,
            row_offset: None,
            max_byte_size: None,
            commit_batch_num: None,
            num_threads: None,
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

impl XmlaOptions {
    /// The property `envelope` travels under in the registered options.
    pub const ENVELOPE: &str = "envelope";
    /// The property `method` travels under in the registered options.
    pub const METHOD: &str = "method";
    /// The property `content` travels under in the registered options.
    pub const CONTENT: &str = "content";

    /// These options as the core carries them: [`RegisteredOptions`] under
    /// the XMLA MIME type, the shared settings as they are and the three of
    /// XMLA's own spelled as properties - `envelope` a flag, `method` and
    /// `content` their names.
    #[must_use]
    pub fn into_registered(self) -> RegisteredOptions {
        let mut registered = RegisteredOptions::new(MimeType::XMLA);
        registered.name = self.name;
        registered.field = self.field;
        registered.filter = self.filter;
        registered.select = self.select;
        registered.merge_by = self.merge_by;
        registered.safe = self.safe;
        registered.batch_byte_size = self.batch_byte_size;
        registered.batch_row_size = self.batch_row_size;
        registered.max_row_size = self.max_row_size;
        registered.row_offset = self.row_offset;
        registered.max_byte_size = self.max_byte_size;
        registered.commit_batch_num = self.commit_batch_num;
        registered.num_threads = self.num_threads;
        registered.level = self.level;
        registered
            .with_property(Self::ENVELOPE, if self.envelope { "true" } else { "false" })
            .with_property(Self::METHOD, self.method.as_str())
            .with_property(Self::CONTENT, self.content.as_str())
    }

    /// The typed options the registered `options` carry: every shared
    /// setting as it is, and XMLA's own read out of the properties - each
    /// absent one its default - a property naming neither read by nothing
    /// here.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] at `$.encoding` for options of
    /// another MIME type, and the vocabulary's refusal for a value no
    /// `envelope`, `method` or `content` spells.
    pub fn from_registered(options: &RegisteredOptions) -> Result<Self> {
        if *options.mime_type() != MimeType::XMLA {
            return Err(not_xmla(options.mime_type()));
        }
        let properties = options.properties();
        let envelope = properties.knob_bool(Self::ENVELOPE)?.unwrap_or(true);
        let method = properties
            .get(Self::METHOD)
            .map_or(Ok(Method::Execute), Method::from_str)?;
        let content = properties
            .get(Self::CONTENT)
            .map_or(Ok(Content::SchemaData), Content::from_str)?;
        Ok(Self {
            name: options.name.clone(),
            field: options.field.clone(),
            filter: options.filter.clone(),
            select: options.select.clone(),
            merge_by: options.merge_by.clone(),
            safe: options.safe,
            batch_byte_size: options.batch_byte_size,
            batch_row_size: options.batch_row_size,
            max_row_size: options.max_row_size,
            row_offset: options.row_offset,
            max_byte_size: options.max_byte_size,
            commit_batch_num: options.commit_batch_num,
            num_threads: options.num_threads,
            level: options.level,
            envelope,
            method,
            content,
        })
    }

    /// The typed options `options` carry, which must be the registered
    /// variant under the XMLA MIME type.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidRecord`] at `$.encoding` naming the encoding
    /// found for any other variant, and what [`Self::from_registered`]
    /// refuses.
    pub fn from_record_options(options: &RecordOptions) -> Result<Self> {
        match options {
            RecordOptions::Registered(registered) => Self::from_registered(registered),
            other => Err(not_xmla(&other.mime_type())),
        }
    }
}

/// The refusal of options describing an encoding other than XMLA.
fn not_xmla(found: &MimeType) -> Error {
    Error::InvalidRecord {
        path: SmolStr::new_static("$.encoding"),
        reason: yggdryl::text::expected_got("XMLA record options", found),
    }
}

impl From<XmlaOptions> for RecordOptions {
    fn from(value: XmlaOptions) -> Self {
        Self::Registered(value.into_registered())
    }
}

impl Default for XmlaOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl IORecordOptions for XmlaOptions {
    yggdryl::record_options_fields!();
}
