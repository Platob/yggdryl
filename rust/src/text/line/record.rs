//! [`TextOptions`], the record-options face of the text-line surface.
//!
//! This is what makes `Text` a *real* media: the same three record methods
//! every encoding answers - [`read_arrow_batch_reader`](crate::io::IOBase::read_arrow_batch_reader),
//! [`write_arrow_batch_reader`](crate::io::IOBase::write_arrow_batch_reader),
//! [`append_arrow_batch_reader`](crate::io::IOBase::append_arrow_batch_reader) -
//! reach the line projection when the options say text, exactly as they reach
//! the IPC decoder when the options say IPC. The extractor itself stays
//! [`TextLineOptions`]; this type adds the shared record settings every
//! [`IORecordOptions`](crate::generic::IORecordOptions) implementation carries.

use smol_str::SmolStr;

use crate::generic::IORecordOptions;
use crate::{Field, Level};

use super::options::TextLineOptions;

/// The record settings of a text-line read or write.
///
/// The extractor - pattern, header, captures, batching, zone - is the held
/// [`TextLineOptions`]; the rest are the settings every record encoding
/// shares, so a text read composes with declared schemas, selections, and
/// partition filters exactly as an IPC or Parquet read does.
///
/// ```
/// use yggdryl::generic::RecordOptions;
/// use yggdryl::io::{Buffer, IOBase};
///
/// # fn main() -> yggdryl::Result<()> {
/// let mut handle = Buffer::new()
///     .with_media_type(yggdryl::Url::from_str("file:///app.log")?.media_type());
/// handle.write_all_bytes(b"first\nsecond\n")?;
///
/// // A `.log` names text/plain, so the record surface reads lines.
/// let options = handle.record_options()?;
/// assert!(matches!(options, RecordOptions::Text(_)));
/// let rows: usize = handle
///     .read_arrow_batch_reader(&options)?
///     .map(|batch| batch.map(|batch| batch.num_rows()))
///     .sum::<Result<_, _>>()?;
/// assert_eq!(rows, 2);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug)]
pub struct TextOptions {
    /// The extractor the lines are parsed under.
    pub lines: TextLineOptions,
    /// Declared canonical schema the projected rows are cast onto.
    pub schema: Option<Field>,
    /// Root Field name used for an inferred schema.
    pub root_name: SmolStr,
    /// Whether a cast may null a value it cannot convert.
    pub safe: bool,
    /// Compression level applied when the handle declares a coding.
    pub level: Level,
    /// Column names forming a write's match key; empty means overwrite.
    ///
    /// A text line has no row identity, so a non-empty key is refused by the
    /// write rather than silently matched against re-parsed rows.
    pub merge_by_names: Vec<String>,
    /// Column names a read or write is narrowed to; empty selects everything.
    pub select_by_names: Vec<String>,
    /// Partition equalities a read is pruned and filtered by; empty keeps all.
    pub filter_partitions: Vec<(String, String)>,
    /// Bound on how many result rows flow in total; `None` is unlimited.
    pub max_row_size: Option<u64>,
    /// Bound on the result rows' Arrow in-memory bytes; `None` is unlimited.
    pub max_byte_size: Option<u64>,
}

impl TextOptions {
    /// Build the default text-record options: one record per line.
    #[must_use]
    pub fn new() -> Self {
        Self::with_lines(TextLineOptions::new())
    }

    /// Build record options around an extractor already in hand.
    #[must_use]
    pub fn with_lines(lines: TextLineOptions) -> Self {
        Self {
            lines,
            schema: None,
            root_name: SmolStr::new_static(super::options::ROOT_NAME),
            safe: false,
            level: Level::DEFAULT,
            merge_by_names: Vec::new(),
            select_by_names: Vec::new(),
            filter_partitions: Vec::new(),
            max_row_size: None,
            max_byte_size: None,
        }
    }

    /// Borrow the extractor the lines are parsed under.
    #[must_use]
    pub const fn lines(&self) -> &TextLineOptions {
        &self.lines
    }
}

impl IORecordOptions for TextOptions {
    fn schema(&self) -> Option<&Field> {
        self.schema.as_ref()
    }

    fn set_schema(&mut self, schema: Field) {
        self.schema = Some(schema);
    }

    fn root_name(&self) -> &str {
        &self.root_name
    }

    fn set_root_name(&mut self, root_name: SmolStr) {
        self.root_name = root_name;
    }

    fn safe(&self) -> bool {
        self.safe
    }

    fn set_safe(&mut self, safe: bool) {
        self.safe = safe;
    }

    /// The row bound is the extractor's own, so there is one source of truth.
    fn batch_size(&self) -> Option<usize> {
        self.lines.batch_size()
    }

    fn set_batch_size(&mut self, batch_size: Option<usize>) {
        self.lines.set_batch_size(batch_size);
    }

    fn max_row_size(&self) -> Option<u64> {
        self.max_row_size
    }

    fn set_max_row_size(&mut self, max_row_size: Option<u64>) {
        self.max_row_size = max_row_size;
    }

    fn max_byte_size(&self) -> Option<u64> {
        self.max_byte_size
    }

    fn set_max_byte_size(&mut self, max_byte_size: Option<u64>) {
        self.max_byte_size = max_byte_size;
    }

    fn level(&self) -> Level {
        self.level
    }

    fn set_level(&mut self, level: Level) {
        self.level = level;
    }

    fn merge_by_names(&self) -> &[String] {
        &self.merge_by_names
    }

    fn set_merge_by_names(&mut self, merge_by_names: Vec<String>) {
        self.merge_by_names = merge_by_names;
    }

    fn select_by_names(&self) -> &[String] {
        &self.select_by_names
    }

    fn set_select_by_names(&mut self, select_by_names: Vec<String>) {
        self.select_by_names = select_by_names;
    }

    fn filter_partitions(&self) -> &[(String, String)] {
        &self.filter_partitions
    }

    fn set_filter_partitions(&mut self, filter_partitions: Vec<(String, String)>) {
        self.filter_partitions = filter_partitions;
    }
}

impl Default for TextOptions {
    fn default() -> Self {
        Self::new()
    }
}

impl From<TextLineOptions> for TextOptions {
    fn from(lines: TextLineOptions) -> Self {
        Self::with_lines(lines)
    }
}
