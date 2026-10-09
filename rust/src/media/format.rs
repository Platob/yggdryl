//! The table formats a container can be laid out as, and the register the
//! record doors read them from.
//!
//! A [`TableFormat`] is one `static` in the format's own crate answering
//! whether a container handle addresses one of its tables and, where it does,
//! the [`LocatedTable`] every record verb then runs through - the schema, the
//! rows, the three writes and the prepared cadences a write session publishes.
//! `locate` is the one question the record doors ask before they treat a
//! container as a folder of leaves; with no format claimed it answers `None`
//! before touching the handle, and a folder laid out as a table is then read
//! as its leaves or refused by the door that knows the layout, naming the
//! crate to install.

use std::fmt;
use std::sync::OnceLock;

use smol_str::SmolStr;

use crate::arrow::BatchReader;
use crate::holder::Holder;
use crate::media::RecordOptions;
use crate::plugin::{CORE, Register};
use crate::{Error, Field, IOBase, IOResult, Properties, Result, Selector, Table};

/// One table format, stated once as a `static`.
pub trait TableFormat: fmt::Debug + Send + Sync + 'static {
    /// The format's name, the key it is claimed under: `iceberg`.
    fn name(&self) -> &'static str;

    /// The table `handle` addresses - the table whole, or one of its
    /// partition directories - if it addresses one of this format.
    ///
    /// # Errors
    ///
    /// Returns an error when a metadata document is found but cannot be
    /// read.
    fn locate(&self, handle: &dyn IOBase) -> Result<Option<Box<dyn LocatedTable>>>;

    /// The warehouse table object this format answers for a folder laid out
    /// as one of its tables, at `path` over the folder's handle `root`,
    /// inheriting its catalog's effective `inherited` properties: what a
    /// folder catalog lists, touching no storage.
    ///
    /// # Errors
    ///
    /// Returns the format's refusal of the handle.
    fn table(&self, path: Vec<SmolStr>, root: Holder, inherited: &Properties) -> Result<Table>;
}

/// A table a container handle addresses, as the record doors drive it.
///
/// What a write's cadences replaced so far is the located table's own state,
/// held for the life of the write session that located it, so a table
/// addressed whole replaces each partition its rows reach once and appends
/// to it after.
pub trait LocatedTable: fmt::Debug + Send {
    /// Whether the location addresses the table whole rather than one of
    /// its partitions.
    fn is_whole(&self) -> bool;

    /// Replace every row of the table with `batches` in one publication,
    /// whatever its partitions.
    ///
    /// # Errors
    ///
    /// Returns a metadata or write failure.
    fn overwrite_whole(&mut self, batches: BatchReader) -> Result<()>;

    /// Empty the table in one publication that keeps it a table.
    ///
    /// # Errors
    ///
    /// Returns a metadata or write failure.
    fn clear(&mut self) -> Result<()>;

    /// The table's stored field, which shapes every chunk of one write.
    ///
    /// # Errors
    ///
    /// Returns a metadata failure.
    fn stored_field(&self) -> Result<Field>;

    /// The table's whole stored root, its metadata whole, from its metadata:
    /// what [`IOMedia::read_origin_field`](crate::IOMedia::read_origin_field)
    /// answers for the container it is located in.
    ///
    /// # Errors
    ///
    /// Returns a metadata failure.
    fn read_origin_field(&self) -> Result<Option<Field>>;

    /// The table's schema under the options' root name, from its metadata.
    ///
    /// # Errors
    ///
    /// Returns a metadata failure.
    fn read_arrow_field(&self, options: &RecordOptions) -> Result<Field>;

    /// The rows the options ask for within the addressed scope, the
    /// options' clauses applied: the reader is complete.
    ///
    /// # Errors
    ///
    /// Returns a metadata or read failure.
    fn read(&self, options: &RecordOptions) -> Result<BatchReader>;

    /// The rows at the addressed location.
    ///
    /// # Errors
    ///
    /// Returns a metadata or read failure.
    fn row_size(&self) -> Result<u64>;

    /// The table's current schema width.
    ///
    /// # Errors
    ///
    /// Returns a metadata failure.
    fn column_size(&self) -> Result<usize>;

    /// The options the table's own rows are read and written with.
    ///
    /// # Errors
    ///
    /// Returns a metadata failure.
    fn record_options(&self) -> Result<RecordOptions>;

    /// Replace the addressed scope with `batches` under `options`.
    ///
    /// # Errors
    ///
    /// Returns a metadata, read or write failure.
    fn overwrite_arrow_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> Result<IOResult>;

    /// Add `batches` under `options`.
    ///
    /// # Errors
    ///
    /// Returns a metadata or write failure.
    fn append_arrow_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> Result<IOResult>;

    /// Merge `batches` into the addressed scope under `options`.
    ///
    /// # Errors
    ///
    /// Returns a metadata, read, merge or write failure.
    fn merge_arrow_reader(
        &mut self,
        batches: BatchReader,
        options: &RecordOptions,
    ) -> Result<IOResult>;

    /// Publish one already-shaped overwrite cadence: the first to reach a
    /// partition replaces it, every later one appends.
    ///
    /// # Errors
    ///
    /// Returns a metadata or write failure.
    fn overwrite_prepared(&mut self, batches: BatchReader, threads: Option<usize>) -> Result<()>;

    /// Publish one already-shaped append cadence, answering the rows it
    /// declined - a key the table already held.
    ///
    /// # Errors
    ///
    /// Returns a metadata or write failure.
    fn append_prepared(&mut self, batches: BatchReader, threads: Option<usize>) -> Result<u64>;

    /// Publish one already-shaped merge cadence on `merge_by`.
    ///
    /// # Errors
    ///
    /// Returns a metadata, read, merge or write failure.
    fn merge_prepared(
        &mut self,
        batches: BatchReader,
        merge_by: &Selector,
        safe: bool,
        threads: Option<usize>,
    ) -> Result<()>;
}

static FORMATS: Register<&'static str, &'static dyn TableFormat> = Register::new("table format");
static SEEDED: OnceLock<()> = OnceLock::new();

/// Claim the core's own formats once, before the register answers anything.
fn seed() {
    SEEDED.get_or_init(|| {
        #[cfg(feature = "iceberg")]
        FORMATS
            .claim(
                crate::iceberg::ICEBERG_FORMAT.name(),
                &crate::iceberg::ICEBERG_FORMAT,
                CORE,
            )
            .expect("the core's own format claims cleanly");
    });
}

/// Claim `format` for the crate `by`, once for the life of the process.
///
/// # Errors
///
/// Returns [`Error::Conflict`] naming the first claimant where the name is
/// claimed already, and [`Error::InvalidRecord`] at `$.encoding` for a claim
/// in the core's own name.
pub fn claim(format: &'static dyn TableFormat, by: &'static str) -> Result<()> {
    seed();
    if by == CORE {
        return Err(Error::InvalidRecord {
            path: smol_str::SmolStr::new_static("$.encoding"),
            reason: smol_str::format_smolstr!(
                "a table format is claimed by the crate that holds it, never as `{CORE}`"
            ),
        });
    }
    FORMATS.claim(format.name(), format, by)
}

/// The format claimed under `name`, if any.
#[must_use]
pub fn format_named(name: &str) -> Option<&'static dyn TableFormat> {
    seed();
    FORMATS.get(name)
}

/// Every claimed format, in name order.
#[must_use]
pub fn formats() -> Vec<&'static dyn TableFormat> {
    seed();
    FORMATS.values()
}

/// The table a container handle addresses, if any claimed format locates
/// one: the one question the record doors ask before treating a container
/// as a folder of leaves. `None` with nothing claimed, the handle untouched.
///
/// # Errors
///
/// Returns a format's refusal of a metadata document it found but could not
/// read.
pub(crate) fn locate<H: IOBase + ?Sized>(handle: &H) -> Result<Option<Box<dyn LocatedTable>>> {
    seed();
    for format in FORMATS.values() {
        if let Some(table) = format.locate(handle.as_io_base())? {
            return Ok(Some(table));
        }
    }
    Ok(None)
}

/// The refusal of a container laid out as a table format no claim reads,
/// at `$.encoding`.
pub(crate) fn unregistered(location: impl fmt::Display) -> Error {
    Error::InvalidRecord {
        path: smol_str::SmolStr::new_static("$.encoding"),
        reason: smol_str::format_smolstr!(
            "`{location}` is laid out as a table format this build does not read; install the \
             crate that claims it and call its `install()`"
        ),
    }
}
