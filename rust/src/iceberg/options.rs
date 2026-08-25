//! One configuration value for a table's commits, writes, and reads.
//!
//! [`IcebergOptions`] is resolved **per field** from three layers, nearest
//! first:
//!
//! 1. an **explicit option** set on the value itself - or stored on a table
//!    with [`Table::set_options`](super::Table::set_options);
//! 2. the **table property** of the same name, falling back to the schema
//!    root's `iceberg:`-prefixed protocol property, exactly as
//!    [`Table::target_file_size`](super::Table::target_file_size) has always
//!    resolved its one key;
//! 3. the documented **default**.
//!
//! A field an explicit option settles never reads its property, so a stored
//! value that does not parse can be shadowed first and repaired after. A
//! property that is present but unparseable is a typed error naming the key
//! and the value, never a silent default.

use smol_str::{SmolStr, format_smolstr};

use super::manifest::FileFormat;
use super::metadata::TableMetadata;
use crate::{Error, Result};

/// Configuration for one table's commit retries, file sizing, and reads.
///
/// The value records only what was set on it; every getter answers with the
/// field's documented default when nothing was. [`Self::for_metadata`] reads
/// the property layer of one table, and [`Table::options`](super::Table::options)
/// resolves all three layers at once.
///
/// ```
/// use yggdryl::iceberg::IcebergOptions;
///
/// # fn main() -> yggdryl::Result<()> {
/// let options = IcebergOptions::new()
///     .with_commit_retries(2)
///     .try_with_read_parallelism(4)?;
/// assert_eq!(options.commit_retries(), 2);
/// assert_eq!(options.read_parallelism(), 4);
/// // An untouched field answers its default.
/// assert_eq!(options.commit_min_backoff_ms(), 100);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct IcebergOptions {
    /// How many beaten commit attempts are retried, when set.
    commit_retries: Option<u32>,
    /// The first retry wait in milliseconds, when set.
    commit_min_backoff_ms: Option<u64>,
    /// The largest retry wait in milliseconds, when set.
    commit_max_backoff_ms: Option<u64>,
    /// The size a data file aims for in bytes, when set.
    target_file_size_bytes: Option<u64>,
    /// How many data files a scan decodes at once, when set.
    read_parallelism: Option<usize>,
    /// How many large-enough files justify a parallel scan, when set.
    read_parallel_min_files: Option<usize>,
    /// The recorded size below which a file does not justify one, when set.
    read_parallel_min_file_size_bytes: Option<u64>,
    /// After how many data commits an automatic compaction runs, when set.
    compact_after_commits: Option<u32>,
    /// The format new data files are written in, when set.
    data_format: Option<FileFormat>,
}

impl IcebergOptions {
    /// Return a deterministic hash of every explicitly configured option.
    pub fn stable_hash(&self) -> u64 {
        crate::stable_hash_of(self)
    }

    /// The property naming how many beaten commit attempts are retried.
    pub const COMMIT_RETRIES_KEY: &'static str = "commit.retry.num-retries";
    /// The property naming the first retry wait, in milliseconds.
    pub const COMMIT_MIN_BACKOFF_MS_KEY: &'static str = "commit.retry.min-wait-ms";
    /// The property naming the largest retry wait, in milliseconds.
    pub const COMMIT_MAX_BACKOFF_MS_KEY: &'static str = "commit.retry.max-wait-ms";

    /// The table property naming the automatic compaction cadence.
    pub const COMPACT_AFTER_COMMITS_KEY: &'static str = "write.auto-compact.commit-interval";
    /// The property naming the size a data file aims for, in bytes.
    pub const TARGET_FILE_SIZE_KEY: &'static str = "write.target-file-size-bytes";
    /// The property naming the format new data files are written in.
    ///
    /// This is the spec's own key, so a table whose property says `avro` writes
    /// Avro data files here exactly as it would under Spark.
    pub const DATA_FORMAT_KEY: &'static str = "write.format.default";
    /// The property naming how many data files a scan decodes at once.
    pub const READ_PARALLELISM_KEY: &'static str = "read.parallelism";
    /// The property naming how many large-enough files justify parallelism.
    pub const READ_PARALLEL_MIN_FILES_KEY: &'static str = "read.parallel.min-files";
    /// The property naming the size below which a file does not count.
    pub const READ_PARALLEL_MIN_FILE_SIZE_KEY: &'static str = "read.parallel.min-file-size-bytes";

    /// The retry count nothing configures: Iceberg's own default of 4.
    pub const DEFAULT_COMMIT_RETRIES: u32 = 4;
    /// The first retry wait nothing configures: 100 milliseconds.
    pub const DEFAULT_COMMIT_MIN_BACKOFF_MS: u64 = 100;
    /// The largest retry wait nothing configures: one minute.
    pub const DEFAULT_COMMIT_MAX_BACKOFF_MS: u64 = 60_000;
    /// The file size nothing configures: Iceberg's own 512 MiB.
    pub const DEFAULT_TARGET_FILE_SIZE_BYTES: u64 = 512 * 1024 * 1024;
    /// The file count nothing configures: 16 large-enough files.
    pub const DEFAULT_READ_PARALLEL_MIN_FILES: usize = 16;
    /// The size floor nothing configures: 4 MiB recorded bytes.
    pub const DEFAULT_READ_PARALLEL_MIN_FILE_SIZE_BYTES: u64 = 4 * 1024 * 1024;
    /// The data file format nothing configures: Parquet, the spec's default.
    pub const DEFAULT_DATA_FORMAT: FileFormat = FileFormat::Parquet;

    /// Build an options value with nothing set, so every field defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// The parallelism nothing configures: what the host offers, kept in 1..=8.
    ///
    /// A machine that will not say how many threads it runs answers 1, which
    /// is the sequential path.
    pub fn default_read_parallelism() -> usize {
        std::thread::available_parallelism()
            .map_or(1, std::num::NonZeroUsize::get)
            .clamp(1, 8)
    }

    /// Return how many beaten commit attempts are retried. Default: 4.
    pub fn commit_retries(&self) -> u32 {
        self.commit_retries.unwrap_or(Self::DEFAULT_COMMIT_RETRIES)
    }

    /// Return the explicitly configured retry count, without applying defaults.
    pub const fn commit_retries_option(&self) -> Option<u32> {
        self.commit_retries
    }

    /// Return the first retry wait in milliseconds. Default: 100.
    pub fn commit_min_backoff_ms(&self) -> u64 {
        self.commit_min_backoff_ms
            .unwrap_or(Self::DEFAULT_COMMIT_MIN_BACKOFF_MS)
    }

    /// Return the explicitly configured minimum retry wait.
    pub const fn commit_min_backoff_ms_option(&self) -> Option<u64> {
        self.commit_min_backoff_ms
    }

    /// Return the largest retry wait in milliseconds. Default: 60 000.
    ///
    /// A configured floor above this ceiling waits the ceiling; the pair is
    /// clamped where the wait is computed rather than refused here, because
    /// each half can arrive from a different layer.
    pub fn commit_max_backoff_ms(&self) -> u64 {
        self.commit_max_backoff_ms
            .unwrap_or(Self::DEFAULT_COMMIT_MAX_BACKOFF_MS)
    }

    /// Return the explicitly configured maximum retry wait.
    pub const fn commit_max_backoff_ms_option(&self) -> Option<u64> {
        self.commit_max_backoff_ms
    }

    /// Return the automatic compaction cadence, when one is set.
    ///
    /// `Some(n)` compacts after every `n` data commits, so small appends fold
    /// into files near the target size without any commit paying for a full
    /// rewrite: a well-paced cadence keeps commits neither so frequent that
    /// every write rewrites files nor so rare that a scan reads hundreds of
    /// undersized ones. `None` - the default - never compacts on its own, and
    /// `Some(0)` reads as off rather than as after-every-commit.
    pub fn compact_after_commits(&self) -> Option<u32> {
        self.compact_after_commits.filter(|cadence| *cadence > 0)
    }

    /// Return the explicit compaction cadence, preserving `Some(0)`.
    pub const fn compact_after_commits_option(&self) -> Option<u32> {
        self.compact_after_commits
    }

    /// Return the size a data file aims for, in bytes. Default: 512 MiB.
    pub fn target_file_size_bytes(&self) -> u64 {
        self.target_file_size_bytes
            .unwrap_or(Self::DEFAULT_TARGET_FILE_SIZE_BYTES)
    }

    /// Return the explicitly configured target file size.
    pub const fn target_file_size_bytes_option(&self) -> Option<u64> {
        self.target_file_size_bytes
    }

    /// Return how many data files a scan decodes at once.
    ///
    /// Default: [`Self::default_read_parallelism`], the host's own
    /// parallelism clamped to 1..=8. A value of 1 is the sequential path.
    pub fn read_parallelism(&self) -> usize {
        self.read_parallelism
            .unwrap_or_else(Self::default_read_parallelism)
    }

    /// Return the explicitly configured read parallelism.
    pub const fn read_parallelism_option(&self) -> Option<usize> {
        self.read_parallelism
    }

    /// Return how many large-enough files justify a parallel scan. Default: 16.
    pub fn read_parallel_min_files(&self) -> usize {
        self.read_parallel_min_files
            .unwrap_or(Self::DEFAULT_READ_PARALLEL_MIN_FILES)
    }

    /// Return the explicitly configured parallel-scan file threshold.
    pub const fn read_parallel_min_files_option(&self) -> Option<usize> {
        self.read_parallel_min_files
    }

    /// Return the recorded size below which a file does not count toward
    /// justifying parallelism, in bytes. Default: 4 MiB.
    pub fn read_parallel_min_file_size_bytes(&self) -> u64 {
        self.read_parallel_min_file_size_bytes
            .unwrap_or(Self::DEFAULT_READ_PARALLEL_MIN_FILE_SIZE_BYTES)
    }

    /// Return the explicitly configured parallel-scan size threshold.
    pub const fn read_parallel_min_file_size_bytes_option(&self) -> Option<u64> {
        self.read_parallel_min_file_size_bytes
    }

    /// Return the format new data files are written in. Default: Parquet.
    ///
    /// Only what a *write* produces is decided here: a scan decodes each data
    /// file as the format its manifest entry records, so one table can mix
    /// formats and still read as one shape.
    pub fn data_format(&self) -> FileFormat {
        self.data_format.unwrap_or(Self::DEFAULT_DATA_FORMAT)
    }

    /// Return the explicitly configured data format.
    pub const fn data_format_option(&self) -> Option<FileFormat> {
        self.data_format
    }

    /// Set the format new data files are written in.
    ///
    /// The build is checked when a write resolves the format, not here, so an
    /// options value can carry a format one build encodes and another refuses.
    pub fn set_data_format(&mut self, format: FileFormat) {
        self.data_format = Some(format);
    }

    /// Set the format new data files are written in, persistently.
    #[must_use]
    pub fn with_data_format(mut self, format: FileFormat) -> Self {
        self.set_data_format(format);
        self
    }

    /// Set how many beaten commit attempts are retried; 0 disables retrying.
    pub fn set_commit_retries(&mut self, retries: u32) {
        self.commit_retries = Some(retries);
    }

    /// Set how many beaten commit attempts are retried, persistently.
    #[must_use]
    pub fn with_commit_retries(mut self, retries: u32) -> Self {
        self.set_commit_retries(retries);
        self
    }

    /// Set the first retry wait in milliseconds; 0 retries immediately.
    pub fn set_commit_min_backoff_ms(&mut self, wait_ms: u64) {
        self.commit_min_backoff_ms = Some(wait_ms);
    }

    /// Set the first retry wait in milliseconds, persistently.
    #[must_use]
    pub fn with_commit_min_backoff_ms(mut self, wait_ms: u64) -> Self {
        self.set_commit_min_backoff_ms(wait_ms);
        self
    }

    /// Set the largest retry wait in milliseconds.
    pub fn set_commit_max_backoff_ms(&mut self, wait_ms: u64) {
        self.commit_max_backoff_ms = Some(wait_ms);
    }

    /// Set the automatic compaction cadence; zero turns it off.
    pub fn set_compact_after_commits(&mut self, commits: u32) {
        self.compact_after_commits = Some(commits);
    }

    /// Return these options compacting after every `commits` data commits.
    #[must_use]
    pub fn with_compact_after_commits(mut self, commits: u32) -> Self {
        self.set_compact_after_commits(commits);
        self
    }

    /// Set the largest retry wait in milliseconds, persistently.
    #[must_use]
    pub fn with_commit_max_backoff_ms(mut self, wait_ms: u64) -> Self {
        self.set_commit_max_backoff_ms(wait_ms);
        self
    }

    /// Set the size a data file aims for, in bytes.
    ///
    /// # Errors
    ///
    /// Returns a typed error naming the key when `bytes` is zero; the value is
    /// unchanged.
    pub fn set_target_file_size_bytes(&mut self, bytes: u64) -> Result<()> {
        if bytes == 0 {
            return Err(Error::InvalidMetadataValue {
                key: SmolStr::new_static(Self::TARGET_FILE_SIZE_KEY),
                reason: SmolStr::new_static("expected a positive byte count, got 0"),
            });
        }
        self.target_file_size_bytes = Some(bytes);
        Ok(())
    }

    /// Set the size a data file aims for, persistently.
    ///
    /// # Errors
    ///
    /// Returns the [`Self::set_target_file_size_bytes`] failure.
    pub fn try_with_target_file_size_bytes(mut self, bytes: u64) -> Result<Self> {
        self.set_target_file_size_bytes(bytes)?;
        Ok(self)
    }

    /// Set how many data files a scan decodes at once; 1 is sequential.
    ///
    /// # Errors
    ///
    /// Returns a typed error naming the key when `threads` is zero; the value
    /// is unchanged.
    pub fn set_read_parallelism(&mut self, threads: usize) -> Result<()> {
        if threads == 0 {
            return Err(Error::InvalidMetadataValue {
                key: SmolStr::new_static(Self::READ_PARALLELISM_KEY),
                reason: SmolStr::new_static("expected at least one reader thread, got 0"),
            });
        }
        self.read_parallelism = Some(threads);
        Ok(())
    }

    /// Set how many data files a scan decodes at once, persistently.
    ///
    /// # Errors
    ///
    /// Returns the [`Self::set_read_parallelism`] failure.
    pub fn try_with_read_parallelism(mut self, threads: usize) -> Result<Self> {
        self.set_read_parallelism(threads)?;
        Ok(self)
    }

    /// Set how many large-enough files justify a parallel scan.
    pub fn set_read_parallel_min_files(&mut self, files: usize) {
        self.read_parallel_min_files = Some(files);
    }

    /// Set how many large-enough files justify a parallel scan, persistently.
    #[must_use]
    pub fn with_read_parallel_min_files(mut self, files: usize) -> Self {
        self.set_read_parallel_min_files(files);
        self
    }

    /// Set the recorded size below which a file does not count, in bytes.
    pub fn set_read_parallel_min_file_size_bytes(&mut self, bytes: u64) {
        self.read_parallel_min_file_size_bytes = Some(bytes);
    }

    /// Set the size below which a file does not count, persistently.
    #[must_use]
    pub fn with_read_parallel_min_file_size_bytes(mut self, bytes: u64) -> Self {
        self.set_read_parallel_min_file_size_bytes(bytes);
        self
    }

    /// Read the property layer of one table's metadata.
    ///
    /// Every field a table property - or the schema root's `iceberg:` fallback
    /// for it - spells is set on the returned value; every other field is left
    /// unset, so its getter answers the default.
    ///
    /// ```
    /// use yggdryl::iceberg::{
    ///     FormatVersion, IcebergOptions, PartitionSpec, TableMetadata, assign_field_ids,
    /// };
    /// use yggdryl::{DataType, Field};
    ///
    /// # fn main() -> yggdryl::Result<()> {
    /// let mut schema: Field = DataType::from_fields([DataType::Int64.required_field("id")])?
    ///     .required_field("row");
    /// assign_field_ids(&mut schema, 1)?;
    /// let mut metadata = TableMetadata::new(
    ///     FormatVersion::V2,
    ///     "file:///tmp/trades",
    ///     schema,
    ///     PartitionSpec::unpartitioned(),
    /// )?;
    /// metadata.set_property(IcebergOptions::COMMIT_RETRIES_KEY, "9")?;
    ///
    /// let options = IcebergOptions::for_metadata(&metadata)?;
    /// assert_eq!(options.commit_retries(), 9);
    /// assert_eq!(options.commit_min_backoff_ms(), 100); // untouched: default
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Errors
    ///
    /// Returns a typed error naming the key and the value when a property is
    /// present but does not parse, and the current-schema failure when the
    /// metadata has no current schema to read the fallback from.
    pub fn for_metadata(metadata: &TableMetadata) -> Result<Self> {
        Self::resolved(None, metadata)
    }

    /// Resolve every field: explicit option, then table property, then unset.
    ///
    /// This is the whole three-layer rule except the defaults, which the
    /// getters supply, so the caller sees which fields were actually
    /// configured. A field `explicit` settles never reads its property.
    pub(super) fn resolved(explicit: Option<&Self>, metadata: &TableMetadata) -> Result<Self> {
        Ok(Self {
            commit_retries: commit_retries_layer(explicit, metadata)?,
            commit_min_backoff_ms: commit_min_backoff_layer(explicit, metadata)?,
            commit_max_backoff_ms: commit_max_backoff_layer(explicit, metadata)?,
            target_file_size_bytes: target_file_size_layer(explicit, metadata)?,
            read_parallelism: read_parallelism_layer(explicit, metadata)?,
            read_parallel_min_files: read_parallel_min_files_layer(explicit, metadata)?,
            read_parallel_min_file_size_bytes: read_parallel_min_file_size_layer(
                explicit, metadata,
            )?,
            compact_after_commits: compact_after_commits_layer(explicit, metadata)?,
            data_format: data_format_layer(explicit, metadata)?,
        })
    }

    /// Resolve only what a commit consults, so a broken read or write
    /// property cannot stop a metadata-only commit from repairing it.
    pub(super) fn commit_settings(
        explicit: Option<&Self>,
        metadata: &TableMetadata,
    ) -> Result<CommitSettings> {
        Ok(CommitSettings {
            retries: commit_retries_layer(explicit, metadata)?
                .unwrap_or(Self::DEFAULT_COMMIT_RETRIES),
            min_backoff_ms: commit_min_backoff_layer(explicit, metadata)?
                .unwrap_or(Self::DEFAULT_COMMIT_MIN_BACKOFF_MS),
            max_backoff_ms: commit_max_backoff_layer(explicit, metadata)?
                .unwrap_or(Self::DEFAULT_COMMIT_MAX_BACKOFF_MS),
        })
    }

    /// Resolve only what a scan consults, the same three-key subset rule.
    pub(super) fn read_settings(
        explicit: Option<&Self>,
        metadata: &TableMetadata,
    ) -> Result<ReadSettings> {
        Ok(ReadSettings {
            parallelism: read_parallelism_layer(explicit, metadata)?
                .unwrap_or_else(Self::default_read_parallelism),
            min_files: read_parallel_min_files_layer(explicit, metadata)?
                .unwrap_or(Self::DEFAULT_READ_PARALLEL_MIN_FILES),
            min_file_size_bytes: read_parallel_min_file_size_layer(explicit, metadata)?
                .unwrap_or(Self::DEFAULT_READ_PARALLEL_MIN_FILE_SIZE_BYTES),
        })
    }

    /// Resolve only the target file size, the field a write sizes files by.
    pub(super) fn target_size(explicit: Option<&Self>, metadata: &TableMetadata) -> Result<u64> {
        Ok(target_file_size_layer(explicit, metadata)?
            .unwrap_or(Self::DEFAULT_TARGET_FILE_SIZE_BYTES))
    }

    /// Resolve only the data file format, the field a write encodes with.
    pub(super) fn write_format(
        explicit: Option<&Self>,
        metadata: &TableMetadata,
    ) -> Result<FileFormat> {
        Ok(data_format_layer(explicit, metadata)?.unwrap_or(Self::DEFAULT_DATA_FORMAT))
    }
}

/// What a commit's retry ladder runs with, fully resolved.
#[derive(Clone, Copy, Debug)]
pub(super) struct CommitSettings {
    /// How many beaten attempts are retried before the commit gives up.
    pub(super) retries: u32,
    /// The first retry wait, in milliseconds.
    pub(super) min_backoff_ms: u64,
    /// The largest retry wait, in milliseconds.
    pub(super) max_backoff_ms: u64,
}

/// What a scan's parallel-read decision runs with, fully resolved.
#[derive(Clone, Copy, Debug)]
pub(super) struct ReadSettings {
    /// How many data files are decoded at once; 1 is the sequential path.
    pub(super) parallelism: usize,
    /// How many large-enough files justify decoding in parallel.
    pub(super) min_files: usize,
    /// The recorded size below which a file does not count, in bytes.
    pub(super) min_file_size_bytes: u64,
}

/// The one resolver for [`IcebergOptions::DATA_FORMAT_KEY`].
fn data_format_layer(
    explicit: Option<&IcebergOptions>,
    metadata: &TableMetadata,
) -> Result<Option<FileFormat>> {
    layered(
        explicit.and_then(|options| options.data_format),
        metadata,
        IcebergOptions::DATA_FORMAT_KEY,
        "a data file format of parquet, avro, or orc",
        |_| true,
    )
}

/// The one resolver for [`IcebergOptions::COMPACT_AFTER_COMMITS_KEY`].
fn compact_after_commits_layer(
    explicit: Option<&IcebergOptions>,
    metadata: &TableMetadata,
) -> Result<Option<u32>> {
    layered(
        explicit.and_then(|options| options.compact_after_commits),
        metadata,
        IcebergOptions::COMPACT_AFTER_COMMITS_KEY,
        "a whole number of commits",
        |_| true,
    )
}

/// The one resolver for [`IcebergOptions::COMMIT_RETRIES_KEY`].
fn commit_retries_layer(
    explicit: Option<&IcebergOptions>,
    metadata: &TableMetadata,
) -> Result<Option<u32>> {
    layered(
        explicit.and_then(|options| options.commit_retries),
        metadata,
        IcebergOptions::COMMIT_RETRIES_KEY,
        "a whole number of retries",
        |_| true,
    )
}

/// The one resolver for [`IcebergOptions::COMMIT_MIN_BACKOFF_MS_KEY`].
fn commit_min_backoff_layer(
    explicit: Option<&IcebergOptions>,
    metadata: &TableMetadata,
) -> Result<Option<u64>> {
    layered(
        explicit.and_then(|options| options.commit_min_backoff_ms),
        metadata,
        IcebergOptions::COMMIT_MIN_BACKOFF_MS_KEY,
        "a whole number of milliseconds",
        |_| true,
    )
}

/// The one resolver for [`IcebergOptions::COMMIT_MAX_BACKOFF_MS_KEY`].
fn commit_max_backoff_layer(
    explicit: Option<&IcebergOptions>,
    metadata: &TableMetadata,
) -> Result<Option<u64>> {
    layered(
        explicit.and_then(|options| options.commit_max_backoff_ms),
        metadata,
        IcebergOptions::COMMIT_MAX_BACKOFF_MS_KEY,
        "a whole number of milliseconds",
        |_| true,
    )
}

/// The one resolver for [`IcebergOptions::TARGET_FILE_SIZE_KEY`].
fn target_file_size_layer(
    explicit: Option<&IcebergOptions>,
    metadata: &TableMetadata,
) -> Result<Option<u64>> {
    layered(
        explicit.and_then(|options| options.target_file_size_bytes),
        metadata,
        IcebergOptions::TARGET_FILE_SIZE_KEY,
        "a positive byte count",
        |bytes| *bytes > 0,
    )
}

/// The one resolver for [`IcebergOptions::READ_PARALLELISM_KEY`].
fn read_parallelism_layer(
    explicit: Option<&IcebergOptions>,
    metadata: &TableMetadata,
) -> Result<Option<usize>> {
    layered(
        explicit.and_then(|options| options.read_parallelism),
        metadata,
        IcebergOptions::READ_PARALLELISM_KEY,
        "a positive reader-thread count",
        |threads| *threads >= 1,
    )
}

/// The one resolver for [`IcebergOptions::READ_PARALLEL_MIN_FILES_KEY`].
fn read_parallel_min_files_layer(
    explicit: Option<&IcebergOptions>,
    metadata: &TableMetadata,
) -> Result<Option<usize>> {
    layered(
        explicit.and_then(|options| options.read_parallel_min_files),
        metadata,
        IcebergOptions::READ_PARALLEL_MIN_FILES_KEY,
        "a whole number of files",
        |_| true,
    )
}

/// The one resolver for [`IcebergOptions::READ_PARALLEL_MIN_FILE_SIZE_KEY`].
fn read_parallel_min_file_size_layer(
    explicit: Option<&IcebergOptions>,
    metadata: &TableMetadata,
) -> Result<Option<u64>> {
    layered(
        explicit.and_then(|options| options.read_parallel_min_file_size_bytes),
        metadata,
        IcebergOptions::READ_PARALLEL_MIN_FILE_SIZE_KEY,
        "a whole number of bytes",
        |_| true,
    )
}

/// Resolve one field's nearest two layers: explicit value, then property.
///
/// An explicit value wins without reading the property at all, which is what
/// lets a caller shadow a stored value that does not parse. `None` means
/// neither layer spoke, and the getter's default answers.
fn layered<T: std::str::FromStr + Copy>(
    explicit: Option<T>,
    metadata: &TableMetadata,
    key: &'static str,
    expected: &str,
    accept: impl Fn(&T) -> bool,
) -> Result<Option<T>> {
    if explicit.is_some() {
        return Ok(explicit);
    }
    match stored(metadata, key)? {
        Some((key, text)) => parsed(key, text, expected, accept).map(Some),
        None => Ok(None),
    }
}

/// Find one property's text: the table property, then the schema root's
/// `iceberg:` protocol fallback, with the key the failure would be named by.
fn stored<'metadata>(
    metadata: &'metadata TableMetadata,
    key: &'static str,
) -> Result<Option<(SmolStr, &'metadata str)>> {
    if let Some(text) = metadata.property(key) {
        return Ok(Some((SmolStr::new_static(key), text)));
    }
    let root = metadata.current_schema()?.iceberg();
    Ok(root
        .get(key)
        .map(|text| (SmolStr::new(root.key(key)), text)))
}

/// Parse one configured value, or say why the text is not one.
fn parsed<T: std::str::FromStr>(
    key: SmolStr,
    text: &str,
    expected: &str,
    accept: impl Fn(&T) -> bool,
) -> Result<T> {
    match text.parse::<T>() {
        Ok(value) if accept(&value) => Ok(value),
        _ => Err(Error::InvalidMetadataValue {
            key,
            reason: format_smolstr!("expected {expected}, got {text:?}"),
        }),
    }
}
